# NEXT PLAN — 2026-08-28 (supersedes 2026-08-24; PRUNED)

## SESSION 2026-09-04 — solver13 icache/fmt residual KILLED (eager verbose formatting); cache-resident cells 1.03-1.06x, 14-cell wide check geomean ~1.02x; PHASE-8 ACCEPTANCE RUN LAUNCHED (paired, 400x2 @ 3600 s / 16 GB / 16+16 pinned cores)

Commits 1276cba (lazy format_args), 7d46c4c (kitten import inline), + this.

**The find**: `print::extremely_verbose/very_verbose/verbose/phase` took a
pre-built `format!` String at ~160 call sites — every restart, every
`kimits::delaying`, every phase formatted floats + malloc'd at verbosity 0
(C's macro tests verbosity first). Fix `format!` → `format_args!` inside
those calls (mechanical script, 158 sites; `very_verbose_if_not_bumpreasons`
now `impl Display`). Paired 3-rep, pinned, idle siblings: SCPC 1.16x →
1.044x, circuit 1.13x → 1.03x, Timetable 1.11x → 1.07x, brocard 1.017x;
icache misses 27.9M → 20.8M (C 10.0M). Counters exact on all cells.

**Where the rest is (Timetable `--profile=2`, both arms)**: search 0.983x
(we are AT the C on search), simplify 1.104x — eliminate 1.161x is +0.57 s
of the +0.88 s total; probe 1.068x (sweep 1.12x, factor 1.11x). Do NOT use
`--profile=4` for this: it doubles both arms' runtime and hides the gap.
Per-function comparisons are muddied by inlining differences (C keeps
`get_ternary_clause`, `kissat_resize_vector` etc. as separate symbols) —
compare phases, or perf-script IP histograms (`-F ip,sym,symoff`; `perf
annotate` on the C binary hangs for minutes). Candidate next targets, each
1-3% of wall: kitten clause import (+11%), `inlined_connect_clause` (2x,
cost on the `*end != INVALID` slot read in push_vectors — same instruction
the C pays, unexplained), `watch_large_clauses` (+15%), vivify (+10%).
Kitten `import_literal` inline(always) + cold enlarge_* was a wash (kept).

**Wide check (step-17b = HEAD 7d46c4c v kissat, 14 medium cells,
`--conflicts=100000`, 7 pairs at a time, cores with idle siblings; scratch
`wideout/`)**: case7 1.029, clqcl_50 1.022, crusti 1.029, DLTM 0.988,
oddball_24 1.000, QG7 1.020, ramsey_3_6_19 1.017, RoundRobin 1.030,
tseitin_grid_n12 1.039, VanDerWaerden_27 1.005, xor_op_n40 1.058,
reconf10_70 1.015, velev-pipe 1.035, sudoku-N30 ~1.0 (173 v 168 s wall,
process-time reversed) — **all 14 counter-exact**, geomean ≈ 1.02x.

**ACCEPTANCE RUN (phase 8) — DONE 16:08 local, GATE PASS**: solved 312 v
313 (floor 306.7), PAR-2 1.0071x (ceiling 1.02x), 0 contradictions, 284
both-solved wall geomean 1.013x; lost only lockchart-group3-L15-K29-p4 (a
53 s wall-coin, kissat 3546.6 s). Report:
`python3 tools/compare_full_runs.py log/kissat-full-accept-20260904-072748
log/solver13-full-accept-20260904-072750`. Residual families: Kakuro
1.15-1.22x (490 MB CNFs — profile parse + giant-clause handling next),
REGRandom 1.15x, crusti 1.11x; `N.normalised` 0.81-0.94x (we are faster).
**Post-acceptance (same day, commits f363ca9..25a4c5c)**: Kakuro profiled
and taken 1.152x → 1.072x (PushCursor hoisting in the watch-stack push
loops, congruence counting passes on literal slices; README steps 14-16);
Timetable now 1.026x. Remaining Kakuro split: congruence 1.13x, preprocess
1.14x, vivify 1.09x, walking 1.15x, search 1.05x. The push-loop residual is
memory-stall time on identical accesses (vectors layout verified line for
line via the `[vectors] enlarged`/defrag phase lines) — no code-shape fix
found; measure with `perf stat -e cycle_activity.stalls_l3_miss` before
trying more. Candidate next: `Flags` is 10 bytes v the C's 2-byte bitfield
struct (var-indexed; 250 field accesses — pack via accessors), vivify
(1.09x on Kakuro), walk's flip loop.
**THE LOAD-SENSITIVITY LAW (later the same day, commits 8d34291..3843da7)**:
the acceptance run's REGRandom 1.15x does not exist on a quiet host (0.98x)
— it appears under a 28-process memory load (1.10-1.14x, core-swap
verified) with IDENTICAL offcore traffic/L2 misses/LLC loads/GHz and a fine
front-end; only `stalls_l3_miss` (+24%) and instructions (+13%) differ.
Same misses, less overlap: extra instructions between misses fill the OOO
window. The acceptance run is a 32-way load, so quiet paired timing
understates the metric-relevant gap. Screen under load: `for i in $(seq 8
35); do taskset -c $i kissat -n -q Kakuro.cnf & done`, pair on cores 2/4.
Fix that proved it: factor's loops as slice walks → instructions 38.4 →
33.9 G (below the C's 34.1), REGRandom 0.92x quiet / 0.96x loaded. Then a
compile-checked crate-wide transform (scratch slicify.py / slicify2.py;
recreate from the README description) rewrote 55 `for i in 0..size {
clause(ref).lit(i) }` loops and 17 `for wi in begin..end { stack[wi] }`
loops as slice walks (circuit −1.8%, SCPC −2.2%); the 25 + 4 sites whose
bodies mutate the arena or call `&mut Solver` methods were left as index
loops. 20-cell discriminating parity at 100k on both transform binaries:
see below.
**Next**: (1) DONE above — Kakuro to 1.07x; (2) the ~1% engines
(sweep/factor/vivify/eliminate connect) — connect + factor done, others at
noise; (2b) loaded-screen the whole 14-cell wide set (RS v C under the
28-process load) to find the remaining load-sensitive engines, then cut
instructions in those loops (the remaining index-loop sites, `ticks`
bookkeeping, Range iterators);
(3) decide what solver13 is FOR now that it is a verified 1.01x kissat
port: the solver12-style feature work (fsweep/chrono/etc.) can be re-based
on it with counter-exact regression testing against the C.
Launch details of the run:
- kissat arm: `log/kissat-full-accept-20260904-072748` (pid of wrapper 245924, `log/accept-kissat-arm.out`),
  `tools/run_kissat_full.sh -t 3600 -m 16000 -j 16 -c 0`.
- solver13 arm: `log/solver13-full-accept-20260904-072750` (wrapper pid 246205, `log/accept-solver13-arm.out`),
  same script with `-k ~/.cache/sat13-accept/sat-solver` (frozen copy of
  HEAD 7d46c4c's release binary, sha256 eafd8dfa37b34306) `-c 16`.
- Both arms simultaneous on disjoint physical cores (kissat: socket-balanced
  order[0..16) = cpus 0-7,18-25; solver13: order[16..32) = cpus 8-15,26-33),
  32 solvers live = the reference methodology's load. No proof emission in
  either arm (kissat CLI form `binary cnf`), `ulimit -v` 16 GB, `timeout
  3600`. Expected duration ~10-12 h. Each arm writes `results.csv` + `DONE`.
- **Harness fix this session** (`tools/run_kissat_full.sh`): pinning by
  `idx % JOBS` doubled two solvers on one core whenever xargs refilled a
  slot with a congruent index (observed live: two kissats on cpu 0); now a
  mkdir slot pool guarantees one solver per core. Added `-k binary` and
  `-n run_name`. The 2026-08-28 reference run had the doubling flaw, so
  compare arms of THIS run only, not against that log.
- The first launch (07:25) was killed for the pinning bug; its partial dirs
  are `log/ABORTED-pinning-*` (left in place — rm needs a human).
- **Evaluation**: gate = solved ≥ 0.98×kissat, PAR-2 ≤ 1.02×kissat, zero
  correctness failures (SAT/UNSAT contradictions between arms; the run
  emits no proofs/models, so re-verify any contradiction cell with
  `run.sh` + drat-trim). Then per-cell wall ratios on both-solved cells,
  and the timeout-band cells (kissat-solved 3000-3600 s) where the
  residual wall gap converts to lost cells.


## SESSION 2026-09-03b — solver13 PROFILER UNLOCKED; the "8.5% gap" was brocard's memory-bound best case (other cells 1.19-1.34x); ten structural fixes take brocard to 1.007x and cache-resident cells to ~1.15x kissat, all counters exact

Commits 7a27474 … 4b0ba3f (nine commits, all pushed).
Host: perf_event_paranoid=1, perf + valgrind present — profiling works now.

**Protocol that produced every number below**: simultaneous pinned-core
runs (`taskset -c N`), candidate v previous step v reference kissat 4.0.4,
each arm's 80 `-s` counters diffed against the C (exact on every step),
brocard full default run + circuit_48in64out and Timetable_C_392 at
`--conflicts=100000`. Decompressed cells and every step's binary are in
this session's scratchpad (`stepN-sat-solver`); `parity.py --solver` now
takes a frozen binary so the tree can keep changing during a 25-min run.

**The finding**: brocard (1M vars, 5.6M clauses) is memory-bound — its
misses hide instruction overhead. `perf stat` at the 4% point: cycles
+3.4%, instructions +15.6%, branches +27%, L1/LLC misses EQUAL. On
cache-resident cells the same overhead is exposed: circuit 1.34x,
Timetable 1.29x, Kakuro 1.29x, REGRandom 1.19x at identical conflict
counts. Cause: checked Vec/slice indexing crate-wide (C indexes raw
pointers under NDEBUG) plus out-of-line helpers the C has as header
`static inline`.

**Fixes, in order (README has the per-step numbers)**:
1. `#[inline(always)]` on the fast-assign chain (assign, fast_binary_assign,
   fast_assign_reference, assignment_level, push_vectors,
   push_blocking_watch, delay_watching_large, watch_large_delayed) — perf
   showed them as separate symbols totalling 24% while the C is one frame.
   brocard −6.3%.
2. `struct assigned` 16 bytes (flags word, repr(C), size guard). −1.7%.
3. sort_literals_inline/move_smallest_literal_to_front inline(always) +
   unchecked; watch_large_clauses word-offset walk; backtrack unchecked
   indexing. −2.2%.
4. PUSH_ARRAY trail push unchecked (resize.rs keeps capacity >= size). noise.
5. substitute_clauses literal loop unchecked. −1.2%.
6. ClauseRef/ClauseMut/arena.clause() unchecked in release (debug-asserted)
   — the universal clause accessor. circuit −3%, Timetable −6%, brocard −2%.
7. **`src/uvec.rs` UVec<T>**: Vec newtype, `[]` unchecked in release, range
   slicing checked, Deref to Vec. values/marks/assigned/flags/links/
   watches/trail/frames/phases/vectors.stack switched — ~800 sites, zero
   call-site edits. Timetable −15%, circuit −7%, brocard → 1.000x.
8. Loop-local arena/values/stack base pointers in propagate_literal only
   (assign keeps &mut Solver). circuit −2.3%, brocard −0.6%.
9. kitten's 8 arrays on UVec. Timetable −4%, circuit −1%.
10. Heap stack/score/pos on UVec + inline(always) heap ops (C inlineheap.h).
   Timetable −9% (1.23x → 1.118x), circuit −1%, brocard 1.001x.
11. kitten klauses on UVec + inline(always) on kitten.c's static-inline
   helpers. Timetable −1.2%, brocard 1.007x, circuit flat.
12. The remaining Solver stacks (analyzed/levels/minimize/poisoned/promote/
   removable/shrinkable/clause/shadow/delayed/etrail/units/sorter) on UVec.
   circuit −1.6%, SCPC −1.9%, Timetable/brocard flat.
REJECTED too: inline(never) on the analyze cluster (deduce/bump/shrink/
minimize/learn) to mirror kissat's no-LTO TU boundaries — icache misses did
NOT drop (27.5M → 28.8M on SCPC), wall a wash.
REJECTED (recorded in README): FAST_ASSIGN-style threading of raw
values/assigned pointers through fast_assign — +2% wall, +1.06%
instructions, worse LLVM codegen. Do not re-add.

**State at handoff**: brocard 1.001x, circuit 1.16x, Timetable 1.12x
(9-process paired run at the heap step; the Timetable arm swings ±0.05x
between runs at this load — read paired ratios within one run only).
circuit `perf stat`: instructions +13.9%, branches +15.7%, cycles +8.3%.
Per-function cycle residual on circuit (RS v C samples): search_propagate
5356 v 4919 (+9%), watch_large_clauses+connect 1115 v 917, kitten 1028 v
850, vivify_round 986 v 852, sparse_sweep 303 v 230; analyze/minimize,
probing_propagate, factor, substitute, forward are at or below the C.
Branch long tail: RS functions under 1.2% carry 20.4k branch samples v
13.8k in C — heap::bubble_up/down/update_heap (1.9k v C's inlined 458),
float formatting of report lines (`float_to_decimal_common_shortest` 0.6%
— C printf is cheaper; only matters with reporting on), alloc/realloc
churn 2x C's.

**Parity**: `parity.py --conflicts 100000` (20 discriminating cells, full
default config): **20/20 on step 5, step 10b, step 11 AND step 14 (= HEAD 4b0ba3f's
binary)** — the tree is at full parity on the discriminating set at 100k
conflicts; every step was also verified 80-counter exact on
brocard/circuit/Timetable/SCPC.

**Wider check (10 medium cells, step 11 v kissat, --conflicts=100000,
identical conflict counts)**: ratios 1.137-1.201 on the nine cells that
reached the limit, geomean **1.148** over all ten (oisc-subrv-and-nested-11
hit the 900 s wall in both arms at 9592 v 9645 conflicts — wall-limited,
not a divergence); the honest number for search-bound cells is **~1.17x**
before steps 11-12 and ~1.15x after; memory-bound giants are at 1.00-1.01x.
SCPC-500-14 perf: instructions +6.6%, branches +14%, icache misses 2.9x,
dcache equal; `search_propagate` instruction count now EQUAL to the C's;
residual = analyze cluster +8% instructions, kitten +25%, sparse collect,
plus front-end pressure whose source is NOT analyze's inlining (tested).
Measurement trap found: pin only on cores whose SMT sibling is idle
(`lscpu -p=CPU,CORE`); a run with siblings busy produced a 1.6x-slow C arm.

**Next**: (1) done — HEAD parity 20/20; (2) the residual is now
per-engine: kitten (its own Vec<Vec<Katch>> watch lists + checked klause
indexing), vivify_round, watch_large_clauses' remaining +20%, the heap
ops (C inlines them into next_decision_variable — `#[inline(always)]` on
bubble_up/bubble_down/update_heap is the obvious try), and propagate's
+9% on cache-resident cells (annotate on the circuit profile, not brocard);
(3) wider paired timing on 10-15 medium cells at --conflicts=100000 before
calling the port "within 2-3%"; (4) then the 400-instance acceptance run
per plan/solver13-port-plan.md phase 8 (`tools/run_kissat_full.sh`
methodology, 3600 s / 16 GB / 32 pinned cores, paired).

## SESSION 2026-09-03 — solver13 sweep-substitute divergence KILLED (kissat's shadowed q-- quirk); 20/20 discriminating parity at FULL DEFAULT config, 10k AND 100k conflicts; brocard full-run 80-counter exact; honest perf baseline ~8.5%

Commits e9e2ed8 (sweep fix), 52e1318 (getrusage clock), + this session's tail.

**The sweep bug**: kissat 4.0.4 `substitute_connected_clauses` ends its
new_size>2 branch with `q--` that decrements a SHADOWED inner lits cursor,
not the outer watch pointer — dead code, so the reference KEEPS a stale
occurrence of the substituted clause in the old literal's list.  Our port
implemented the intended move semantics → 126 extra live-watched clauses,
+5 probing_ticks drift on brocard.  Fix: drop our `q -= 1` (PORT NOTE in
src/sweep.rs).  Isolation method that worked: SWEEPDBG watch-list hash
dumps at sweep boundaries → per-round GDBG garbage-mark streams with GSITE
call-site tags → per-ref RDBG occurrence counts.  ~2.5 min per paired
brocard rerun; whole chase ≈ one session.

**Parity state (all tier-1)**: smoke corpus exact; discriminating 20/20
exact at `--conflicts=10000` AND `--conflicts=100000`, FULL default config
(all inprocessing live); brocard full unlimited run to UNSAT exact on all
80 counters.  Next parity escalations: (a) full-run parity on more
fast-solving cells, (b) medium-suite spot cells, (c) shuffled inputs.

**Perf state**: honest gap ~8.5% wall on brocard (search +6%,
probe/simplify/vivify/sweep +20-25%, parse 1.19x).  The 10x/7x/4x phase
ratios previously seen at --profile=4 were artifacts of process_time()
parsing /proc/self/stat per START/STOP — now libc getrusage (52e1318).
REJECTED with paired evidence: next-clause software prefetch in
propagate_literal (+3.4%); watch_large_delayed empty-guard (wash).
KEPT: #[inline(always)] on propagate_literal (matches C textual
inlining; ~1%, within noise).  **Blocked on real profiling**: host has
perf_event_paranoid=4 and no valgrind — ask user for
`sudo sysctl kernel.perf_event_paranoid=1` or `sudo apt install valgrind`
before more propagate work; without it, micro-opts are dart-throwing.

**Standing next steps**: (1) unlock profiler, close the 8.5% (propagate
first — 74% of runtime); (2) longer/full-run parity escalations above;
(3) once wall is within ~2-3%, first sat-comp-2025 400-instance
acceptance run per plan/solver13-port-plan.md (3600s/16GB/32j paired
vs kissat).

## SESSION 29 (2026-08-26..28) — faithful kissat sweep.c port BUILT + BANKED default-OFF (ranked item 1 closed as an ENGINE); uniqinv40 + b18 FIRST-EVERS delivered standalone; NO default promotion (armed gate 293 v 297); the inline-tag soundness contract found and fixed; the MODEL-AUDIT debug method added to the toolbox

**What was built (commits 3abd708, be91624, + this): the no-more-nibbles
full sweep.c port, `src/sweep_kissat.rs` + kitten upgrades, behind
`SAT_SWEEP_FAITHFUL=on|armed|yield` (default OFF, flag-off byte-identical
— rbsat fingerprint digit-exact).** Per-variable kitten environments off
a persistent occurrence-sorted doubly-linked schedule (incomplete
schedules resume across calls; completed passes double env limits
256→8192 vars / 1024→32768 clauses / depth 2+c cap 3), backbone +
equivalence partition refined by models and kitten flip pre-tests
(sweepfliprounds=1), equivalences PROVEN by paired implication solves
and substituted into the REAL clause DB IMMEDIATELY
(substitute_connected_clauses parity: in-place rewrite, tautology
delete, unit-break, definition binaries kept, occ lists maintained,
repr union-find), kissat effort budget (100‰ of search ticks since last
call, floor 10M kitten ticks) + BUMP/REDUCE delay throttle, and
core-lemma-only RUP proof emission (kitten tracks antecedent closures;
lemmas emitted once, ascending). Kitten gained faithful-mode-gated
COMPLETE assumption handling (the legacy one-shot install can return
Sat on assumption-UNSAT after backjumping past the assumption — a
measured yield hole), per-solve phase randomization, status/fixed_lit/
core_learned accessors, plus an adversarial usage-pattern fuzz.

**THE ACCEPTANCE RESULT: uniqinv40prop CONVERTED FIRST-EVER — UNSAT in
136 s (global), 170 s / 558 s (armed variants), 754 s in-gate; kissat
51 s; proof drat-trim VERIFIED every time.** 1,594 equivalences + 56
units with 43k in-place rewrites collapse it; CDCL refutes in 0.9-3.4M
conflicts. The S20 arc's residual is now PROVEN to have been the
apply-after-round shape (ELS-deferred substitution), not environment
reach or kitten throughput: immediate substitution alone sustains the
cascade at HALF kissat's equivalence count. **b18 (BMC, kissat-only
since forever) also CONVERTED FIRST-EVER under the global form (UNSAT
3,126 s screen / 3,477 s 3-arm screen; needs the root-sweep head start
— the armed scope misses it).**

**THE GATE (log/abtest-fsarmed-vs-base-2026-08-27-09-09-03, 400x2 @
3600 s/16 GB/32 cores, armed scope): 293 v 297, ZERO correctness
failures, judged NOT PROMOTABLE.** Gained: uniqinv40 (754 s), TT495
(SAT 2,858 s — kissat times out), lockchart-L190, ncc_2_18 + valves
coins. Lost: Circuit24 (2,864 s margin), traffic_kkb (2,488 s), TT496
(2,468 s — TT-family swap with TT495, net 0), dislog (1,079 s),
BvP_7_6 (932 s), ncc_21015 (489 s coin), m29 (450 s in-band reroll —
fsweep m29 standalone is FASTER: 2,386 s v the 3,166 s strict-auto
bank), sqrt169 (444 s flipper), lockchart-L210 (88 s coin/family).
Tier-2: conflicts −3.76% over 288 both-solved (219 conflict-identical),
oski15 family −30-50%, bv_ILA −63%, VexRiscv −54%, sted2 −63%,
case11/grs/DLTM big wins; PAR-2 LOSE 936k v 916k.

**THE LAW (fresh data, joins S16-reuse/S21-parity/S27-escalation): the
faithful sweep's per-cell sign is deterministic with NO clean runtime
discriminator on the armed surface.** Measured discriminator attempts:
(a) per-round yield latch — uniqinv40 finds only 375/round, never
fires; (b) CUMULATIVE old-sweep equivalences — orders WRONGLY
(uniqinv40 641 < Circuit24 1,162 at a 2.5M-conflict horizon; casualties
BvP 0, sqrt169 109, m29 168 — but Circuit24 poisons any threshold);
(c) walk_steps — no separation (uniqinv40 walks 1.35G steps, dislog
2.88G, Circuit24 1.76G); (d) congruence merges — 0 on winners AND
casualties. The winners' fsweep yields are 1,594+ where the casualties'
are 8-58, but that is only knowable AFTER paying the reroll.
Global-scope screen (log/abtest-fs-vs-base-2026-08-27-01-14-23) loses
the walk bank wholesale (dislog/rbsat/TT496/Circuit24); armed scope
(log/abtest-fsa-vs-fsg-vs-base-2026-08-27-03-44-12) recovers Circuit24
in-screen but not in-gate. Do NOT re-gate a scope variant without a NEW
discriminating axis; the engine + per-cell map wait in tree.

**THE SOUNDNESS FIND (fixed; a standing contract for ANY mid-search
formula editor): inline-binary-tag safety.** Tagged binary watchers
(SAT_WATCH_INLINE_BIN) are trusted BLINDLY by propagation — no arena
validation, that is the whole optimization. A clause rewritten in place
under a live tagged watcher leaves a stale entry that FALSE-PROPAGATES;
the guarded-chrono assignment-level computation (max over antecedent
levels) then read a stale unassigned var's decision_level=0 and minted
a PERMANENT poisoned "root" value (survives every backtrack) → false
UNSAT on dislog. Base is safe by construction (tags activate only on
never-arming cells that never edit); the sweep violated the contract.
Fix: `deactivate_watch_inline_tags` — the faithful sweep strips all tag
bits before its first edit; lazy untagged validation absorbs any
staleness; never-swept cells keep the tagged path byte-identically.

**THE METHOD (add to the toolbox, next to S27b wall-band profiling):
REFERENCE-MODEL AUDITING.** Debug an intermittent false-UNSAT by (1)
solving the cell once at base defaults and saving the model; (2)
re-running the candidate with `SAT_DEBUG_MODEL_FILE=<model>` — every
recorded proof clause is checked against the model and the FIRST false
derivation panics with a backtrace naming the pass; (3)
`SAT_DEBUG_PROOF_CLAUSE=<lits>` backtrace-hooks any specific recording;
(4) `SAT_DEBUG_FSWEEP_VARS=<vars>` logs sweep facts/rewrites touching
listed vars. This localized the tag bug through FOUR layers (vivify →
search learning → poisoned trail → stale tagged watcher) in one
evening; drat-trim forward mode alone had stalled at
true-but-unverifiable lines. Also banked: binary-DRAT python parsing
snippets and the earliest-false-clause model-scan (scratch).

**Remaining-gap aggregate (armed-gate deal, base arm vs kissat
08-10): base 297 v kissat 294, we-only 43, kissat-only 40, both-timeout
63.** kissat-only 40 = 6 mapped miters (bit27/28) + 5 BMC (b18, b19_1,
SAT_dat.k100, pj2008, pj2016) + 2 giant steps (nla-dijkstra, x-epic) +
2 grs + structural singletons (uniqinv40*, myciel6, mod4block,
fixedbandwidth, goldcrest, oisc, SGI_30, cfi-rigid, rook-51, par32-2,
BvP_8_4) + lottery tail (HCP-446, case6, ER_400, oddball x4, lockchart
x2, Timetable x2, valves/ncc coins, bp4). *uniqinv40 and b18 are now
OURS-capable (fsweep standalone) — the first cracks in the kissat-only
core; they need the discriminator, not new mechanisms.

**The find (gdb-parent leaf profile, 400 samples, symbolized binary):
WatchPool::push = 10% of m29's wall** — the swap_remove+push pair runs
on every long-clause watch move, and push carried a double meta load
plus a checked slot write. Fix (commit 4a3207d): one meta load + one
unchecked write under the materialization invariant (grow() eagerly
resizes data to start+cap); swap_remove unchecked under pos<len<=cap.
Order-preserving ⇒ trajectory-identical BY CONSTRUCTION.

**Paired quiet: m29 1M-conflict −4.3%, rbsat 100k −3.7% (conflicts
digit-identical). Gate (frozen-snapshot 51579f2 arm,
`log/abtest-poolfast-vs-s28old-2026-08-25-23-30-47`, 400x2 @ 3600 s):
PASS, WIN 293 v 292 — 291/291 both-solved cells conflict-IDENTICAL,
wall −1.52% in-contention, zero correctness failures. +cfi-rigid-t2
(SAT 3,340 s — the S27 one-cell prize, now converted by pure speed)
+sqrt-mitern169 (3,418 s); −ncc_21015 (base margin 30 s,
identical-trajectory wall coin).** The deal itself was weak (both arms
in the low 290s — deal variance); the paired signal is the verdict.

Also closed this arc: kitten throughput measured 9.3 µs/solve on
uniqinv40 — 2x FASTER than kissat's 18 µs (S20b's "kitten throughput"
chunk is NOT a lever); wall band fully search-bound (>=99.5%
search_sec on 8 band cells). Snapshot dir solver/00-s28-snapshot/
(untracked) holds the frozen 51579f2 binary; recreate the temporary
CONFIG_MAP entry on demand (NOT committed).

## SESSION 28 FINAL (2026-08-25) — SAT_CHRONO_STRICT=auto PROMOTED (band-scoped faithful kissat chrono); auto gate 297 v 298 judged promotable (single delta = valves wall coin, 24 s margin; tier-2 −0.63%, wall −2.26%, PAR-2 win, 268/297 conflict-identical); m29 coin CONVERTED to 434 s margin

**Promotion (commit c50de9f):** `SAT_CHRONO_STRICT=auto` default on.
Scoped gate `log/abtest-auto-vs-base-2026-08-25-05-19-21` (400x2 @
3600 s/16 GB/32 cores): auto 297 v base 298 — mechanical line FAIL,
judged PROMOTABLE under "Judging Trades": the ONLY solved delta is
valves-gates (base 3,576 s = 24 s margin, test-1 wall coin, documented
coin list, flipped IN/OUT across the last three deals; it IS in scope —
28,001 merges — so expect it to flip back on quiet deals), priced
against mechanism gains: tier-2 conflicts −0.63% (594.4M v 598.2M),
both-solved wall −2.26%, PAR-2 916,835 v 917,264, zero correctness
failures/contradictions, and 268/297 both-solved cells
conflict-IDENTICAL (the out-of-scope surface untouched BY CONSTRUCTION;
rbsat fingerprint digit-exact). In-scope wins: **m29 converted from
base's 6 s-margin lottery scrape to 3,166 s / 434 s margin with 1.03M
fewer conflicts — strict solves m29 in 3 independent deals (screen
3,443 s, unscoped gate 3,028 s, auto gate 3,166 s); the S26 in-band
coin is now a banked capability.** bwo −193 s (6.83M v 7.31M conf),
oski15 family (12 cells, congruence-productive) up to −1,072 s,
VexRiscv −151 s / −7% conf, ibm-2004 −16 s / −23% conf. Both arms at
record counts (base 298 = highest single-arm ever; the S27b margin
conversion keeps paying).

**The three-deal record lineage this session: 296 (S27b) → 297/298 —
the S27b prediction ("median deals should read ~296-297") confirmed.**

### SESSION 28b (2026-08-25 evening) — two follow-up arcs measured NEGATIVE and banked; the remaining-gap aggregate rebuilt from the freshest 400-cell data

1. **Strict-chrono x armed-restart-cadence coupling: KILLED** (details in
   ranked item 2 below — ibm +79% / vex +9% / oski06 +15% conflicts;
   x-epic stops OOMing but does not convert).
2. **SAT_SWEEP_OCC_MERGE (kissat substitute_connected_clauses reach
   analog): KILLED as default, knob banked off (commit 8925f63).**
   uniqinv40 acceptance failed — 1,642 equivalences with the merge v
   1,655 without: the plateau is IDENTICAL, so environment reach is NOT
   the sweep residual (re-confirms S20 finding 3 from a new angle;
   kissat reaches 3,799). And dislog_a14 TIMED OUT under the merge
   (base SAT ~2,430 s) — the standing fragile-bank kill. HCP-446 par
   (2,680 s, 1,880 equivs). What remains of the uniqinv40 arc, sharpened:
   the residual is kissat's PER-SWEEP CANDIDATE MECHANICS + its
   continuous ~23k-conflict sweep cadence + the substitute→congruence
   re-extract interleave on the REAL clause DB — a full faithful
   sweep.c port, not a reach or environment tweak. Three nibbles have
   now failed (S20b wide-envs, S20b flips, S28b occ-merge): do not
   nibble a fourth time.

3. **Wall-band tax hunt: CLEAN (the S27b method re-applied).** 8 wall-band
   cells (dmu28/bvp76/ncc21015/goldcrest/rr17/mvrr/bivium/sqrt169) at an
   1800 s stats horizon: search_sec >= 99.5% of elapsed on every one,
   sweep_prove <= 32 s, unexplained share <= 0.7%. No hidden pass tax
   remains post-S27b — the band is pure CDCL search; further wall gains
   need engine or trajectory work, not pass fixes.
4. **Mapped-miter upside map: NOT close.** The 4 unsolved 16_16
   bit27/28 MAPPED variants all dive2-latch and get the full
   strict+dive2+elim stack, and still run 12.5-13.4M conflicts at
   3600 s QUIET without solving — unlike pre-conversion m29 (which
   solved standalone at 2,269 s). The mapped-family residual is
   composite throughput, not a missing latch.

**Remaining-gap aggregate (auto-gate union vs kissat 08-10 same-host):
our union 298/400 v kissat 294; we-only 43, kissat-only 39,
both-timeout 63.** The kissat-only 39 decompose: 7 additional 16_16
miter bit27/28 MAPPED variants (in-band strict-auto rolls — upside),
5 BMC (b18, b19_1, SAT_dat.k100, pj2008_k200, pj2016_k100), 2 giant
steps (nla-dijkstra, x-epic — strict helps, scope misses, see item 3),
2 grs, ~11 structural singletons (uniqinv40, myciel6, mod4block,
fixedbandwidth, goldcrest, oisc, SGI_30, cfi-rigid, rook-51, par32-2,
BvP_8_4), and ~12 lottery/deal cells that swap sides across deals
(HCP-446, ncc, case6, oddball residue, ER_400.apx_2, lockchart,
Timetable, bp4).

### S28 evidence trail (unscoped arm + probes + screen)

**Unscoped gate `log/abtest-strict-vs-base-2026-08-24-14-46-54` (400x2 @
3600 s/16 GB/32 cores): strict 284 v base 297 — LOSE.** Zero
correctness failures; 1 crash (x-epic OOM = UNKNOWN_rc-6, priced).
**The per-cell map is the deliverable:** GAINED 8 = m29 (UNSAT 3,028 s
— the S26 in-band coin, ALSO solved 3,443 s in the screen deal: strict
converts it reliably) + 7 lottery coins (vmpc, oddball_56, lockchart-
L220, 6g_6color, ncc, Circuit_mult29, bp4_TCO_IXA). LOST 21 = the SAT
walk-lottery bank almost wholesale (5 oddball_tto/ttf, 2 lockchart, 2
sum-of-cubes, rbsat, dislog, ER_400, ITC_Early_12, VdW-27, frb80, fsf,
sted2, velev-pipe-sat, 2 bp4-SAT, valves+bp4_BC012 near-wall UNSAT).
Var-count does NOT separate (lockchart-L220 gained at 2.25M vars,
L210 lost at 2.05M — family lottery). The winners' shared trait is
gate-rich UNSAT-grind structure (miters=dive2 band; ibm/vex/nla/pj BMC),
the losers' is the gateless walk/rephase lottery surface. **Same law as
S21 restart-parity and S27 escalation: global trajectory changes forfeit
the SAT bank. Band-scope it.**

**Scope-signal truth table (measured, scratch probes):** ALL 10 probed
unscoped-gate losses have ZERO congruence merges and no dive2 latch
(dislog, velev-pipe, bp4_TCO_CSO, ITC, ER_400, fsf, oddball_51, rbsat,
sted2, sum_of_3_cubes) — out of scope, recovered byte-identical. The
winners latch hard: ibm-2004 merges=144,967 (armed at conflict 1),
VexRiscv 18,360, x-epic 50,166, valves 28,001; m29/bwo via dive2
band 2. **nla-dijkstra and pj2016 have ZERO merges — the two giant BMC
step cells with measured strict upside (3x throughput / halved learned
lits) are OUTSIDE the auto scope**; they need a different discriminator
(future arc below). ncc_none_2_18: 0 merges — its unscoped-gate 233 s
solve was global-trajectory luck on a documented coin cell.

Commits: 62f865d (groundwork, off default), 1b843d9 (auto scope),
c50de9f (promotion, auto default).

**What was built (ranked item 1, the S27 prerequisite):** all 5 measured
divergences from kissat chrono closed, behind SAT_CHRONO_STRICT=on
(+SAT_CHRONO_STRICT_LEVELS, default 100 = kissat chronolevels):
(1) determine_new_level parity — backjump >levels ⇒ backtrack exactly ONE
level, no asserting guard (shipped guard delta=5000 never fired);
(2) one_literal_on_conflict_level reuse — single-conflict-level conflicts
become their own driving clause, no learning/bumps/glue-EMA + kissat's
two-highest-level watch repositioning on every long conflict;
(3) learn_unit parity — learned units go through determine_new_level(0),
assigned level-0 OUT OF ORDER on deep trails, absorbed into the root
prefix at the next backtrack-to-0 (this is the big one on unit-rich BMC);
(4) reuse-path cadence ticks conflicts+level-EMA, not glue EMAs.
761+5 tests, smoke 9/9, regrandom strict UNSAT proof drat-trim VERIFIED
(1,207 strict backtracks + 1,911 reused conflicts exercised in debug).

**Tier-1 probes (quiet paired, scratch):** ibm-2004 −23% conflicts
(368,273 v 479,896) −13% wall — the July "delta-100 derails ibm" verdict
belonged to the UNfaithful port; VexRiscv −6.7% conf (2,775,717 v
2,975,066 digit-exact base) −8.5% wall; nla-dijkstra_step 3x conflict
throughput at 3600 s (493k v 163k) with learned length HALVED (26 v 53
avg lits); pj2016 +7% conf, learned lits halved; **x-epic_p16_step
PATHOLOGICAL — avg learned clause 879 lits (v base 161), 30x
ticks/conflict, OOM-abort at 2,320 s (7.3 GB alloc under 16 GB cap;
lands as UNKNOWN_rc134 = priced unsolved, NOT a gate correctness fail).**

**Why kissat survives x-epic at chronolevels=100 (measured, kissat -s):**
its chronological rate there is IDENTICAL to ours (28% of conflicts) but
it restarts every 34 conflicts v our 230 — the frequent-focused-restart
cadence resets the deep trail so cones stay glue-2/3. The x-epic
pathology is strict-chrono x TAME-RESTART interaction, not chrono
itself. UNMEASURED COMBINATION for a future arc: strict chrono +
kissat-parity restart cadence (S21's restart-parity negative predates
strict chrono; kissat couples the two by design).

**Tier-2 screen (benchmarks/chronoscreen-2026-08-24, 15 cells = 5
mechanism + 10 casualty canaries, 4 arms,
log/abtest-strict100-vs-strict300-vs-strict1000-vs-base-2026-08-24-11-34-52):**
strict100 9/15, strict300 8/15, strict1000 11/15, base 12/15.
strict100 wins the grind — m29 UNSAT 3,443 s IN-CONTENTION (the S26
in-band coin banked!), bwo −182 s, MVRR −656 s, manthey −86 s, ibm −12 s
— and loses the SAT walk bank: rbsat/sum_of_3_cubes/dislog TIMEOUT,
TT496 +442 s, oddball_19_4 +549 s. strict300 strictly worse. strict1000
≈ base+noise (fires too rarely; dislog −310 s its only distinctive win).
Screen is deliberately casualty-stacked and contended — the 400-cell
gate decides; even a FAIL yields the full casualty map for band-scoping.

One-file plan for the next clear context. SESSIONS 4-13 bodies live in git
history (`git log -p plan/next-plan.md` up to 52a8f95); SESSIONS 14b/14c/14d
bodies were pruned earlier — full text in revisions up to 93ab682. Where this
file contradicts an older revision, THIS file wins.

**START HERE:** read "SESSION 28 FINAL" (faithful kissat chrono port —
scoped SAT_CHRONO_STRICT=auto PROMOTED; the unscoped 284-v-297 negative
and the restart-cadence coupling discovery), then "SESSION 27b" (the
sweep-prover quadratic KILL — promoted, record 296/400), then
"SESSION 27" (the kissat
causal-ablation GRID + the decisive unscoped-escalation NEGATIVE),
then "SESSION 26"
(dive2-scoped elimination-bound escalation PROMOTED — the
causal-ablation method that found it), then "SESSION 25" (dive-scoped
trail reuse), then "SESSION 24 NEGATIVES", then "SESSION 23" (engine
speed — the lit_vals mirror), "SESSION 22" and "SESSION 21" (the
dive-restart latches), then "RANKED PLAN", then "Standing traps".

## SESSION 27b (2026-08-23/24) — sweep-prover QUADRATIC KILLED, PROMOTED (flagless, identity-proven); NEW ABSOLUTE RECORD 296/400 both arms; the 3,100-3,500 s coin band converted to solid margin

**The find:** gdb-as-parent sampling (ptrace_scope=1 workaround; sampler
in scratch, method now proven) on wall-band cell bp4_BC012_CSO put 89%
of its wall (134/150 samples) inside `sweep::prove_facts_budgeted_opts`
— the model store was Vec<Vec<bool>> full snapshots, rescanned per
backbone candidate AND per equivalence pair (O(n² pairs × #models),
models growing on every kitten flip). This engine runs on EVERY
yield-armed sweep cell (the S20 latch class).

**The fix (commit 5027737):** incremental partition refinement over
XOR-normalized model signatures — `same_as_m0` answers the backbone
question, `class_id` equality answers the pair question, O(1) each,
O(n) refinement per new model, no stored models. Identical booleans at
every decision point ⇒ identical kitten call sequence ⇒ bit-exact
yields and trajectories. Flagless (identity-proven, S23 lit_vals
precedent).

**Paired quiet identity+speed proof:** bp4 conflicts/backbones/equivs/
solves ALL digit-identical (1,508,168 / 11 / 373 / 252,872); sweep-
prove wall 1,650 s → 60 s (27.5x); bp4 total 2,415 → 759 s (−69%).
dislog 1,547 s on its digit-exact 4.94M trajectory (−45% v in-gate
2,825 s). HCP-446 2,568 s standalone (−4%; small sweep share).

**Gate (frozen-snapshot method, pre-fix 95f6289 binary as baseline arm;
log/abtest-sweepfix-vs-s27old-2026-08-23-10-44-21, 400x2 @ 3600 s/
16 GB/32 cores): PASS. 296 v 296 solved — BOTH ARMS the highest count
ever recorded on this bench (previous best 294) — with ALL 295
both-solved cells conflict-IDENTICAL (100%), PAR-2 915,096 v 932,820
(−1.9%), zero correctness failures.** Median wall −4.9% (mean −6.1%)
across the 182 identical-trajectory cells >100 s. Monster margins:
manthey 3,256→694 s (−79%), bv_ILA_Piccolo_JALR 2,377→581 (−76%),
bp4_CSO 3,431→1,367 (−60%), oski15 −35%, sted1 −28%, grs-32-64 −26%,
oddball_29/19 −26/−16%, bp4 family −22-23%, RR_n17 3,110→2,868. Wall
coins +reconf10_22 / −valves (net 0). dmu28 reproduced in-gate
(2,453 s). The band cells that flipped as coins in every recent deal
(manthey/bp4/bwo/sqrt169/RR_n17) now carry 400-2,900 s margins —
solved-count on median deals should read ~296-297.

**Method law (add to the toolbox):** profile the WALL BAND, not just
the gap cells — a quadratic in a niche pass taxed 15+ solved cells
invisibly for months (sweep_prove_nanos existed as a stat all along;
nobody compared it to wall). Check `*_nanos` stats against elapsed
before hunting micro-optimizations.

## SESSION 27 (2026-08-22/23) — NO PROMOTION: the kissat mechanism GRID mapped (the session's durable deliverable); unscoped COMPLETE-round escalation measured a decisive bench-scale LOSER (call 286 / cnod 285 v base 291); two standalone first-evers banked

**The grid (reuse it before believing any "needs a port" claim):** 16
kissat-only cells x 12 kissat single-mechanism ablations
(scratch grid_results.tsv; key rows preserved here). ELIMINATE depth is
load-bearing on 9+ cells: b18 / grs-32-128 / pj2016 noelim=TIMEOUT,
SAT_dat 4.8x, goldcrest 2.4x, myciel6 1.8x, oisc/x-epic 1.5x. VIVIFY
secondary (fixedbandwidth 3.0x, ncc_21015 2.6x, goldcrest/oisc 2.0x,
b18 1.9x). CHRONO: nla-dijkstra 2.9x, x-epic 2.2x, pj TO — but OUR
guarded SAT_CHRONO=on does NOT reproduce it (4 standalone probes all
timeout; a faithful chrono re-port is the prerequisite to even test).
uniqinv40 = nosweep 28.5x + nosubst 4.1x (the S20 sweep-port arc,
confirmed and sized). pj2016 is composite (noelim AND nocongr AND
nochrono all TO). oddball_24_4: every kissat mechanism REMOVED makes
kissat FASTER (novivify 0.1x) — over-inprocessed there; not our story.

**The negative (do not re-run — this closes the July question at bench
scale):** SAT_ELIM_BOUND_COMPLETE_ALL (complete-round escalation for
every armed round; b18's congruence-armed trace reproduces the S26
bve_grow=0 stall verbatim, and the mechanism probe was spectacular —
bound 16, 85% eliminated, wall −34% at 2M conflicts). Standalone it
converted ncc_21015 (2,714 s) and grs-32-128 (3,606 s) FIRST-EVER, and
in-gate it converted cfi-rigid-t2 (2,893 s, first-ever). But the 3-arm
gate (log/abtest-call-vs-cnod-vs-base-2026-08-22-12-20-56, 400x3 @
3600 s/16 GB/32 cores) is unambiguous: call +2/−7 = 286, cnod (TT
shielded) +1/−7 = 285, base 291. The losses are the banked fragile
surface itself: VexRiscv (base 2,131 s FAT margin — the wire-cell
casualty July predicted), dislog, sqrt-mitern169 (the S26 capture),
TT496 (call only — the cnod shield DID work), goldcrest-11, ncc-2_17,
RR_n17/bp4 coins. 64/284 both-solved cells conflict-differ (v S26's
1/286). **Law: complete-round escalation is shippable ONLY inside
narrow structural bands; the armed surface at large carries too many
banked captures. The knob stays in tree (on|nodecision) for future
band-scoped reuse.** ncc/grs remain standalone-only capabilities
(wall-marginal in-gate); cfi-rigid-t2 is a one-cell prize with no
structural band yet — do NOT gerrymander one.

Also closed this session: the unarmed-inprocessing hypothesis for the
BMC class — b18 arms at 200k conflicts, SAT_dat at 1, goldcrest arms
too; the flywheel never fires there. Flywheel vivify + binfrac-ceiling
knobs (SAT_UNARMED_FLYWHEEL_VIVIFY / _MAX_BINFRAC) are in tree,
default-inert. TT392 canary byte-identical-fast under escalation.

## SESSION 26 (2026-08-21/22) — dive2-scoped kissat elimination-bound escalation PROMOTED: gate WIN 291 v 288 (PASS, zero correctness failures); dmu28 FIRST-EVER; the elimination-depth gap on miters is CLOSED at its root

**The method that cracked it (reuse this): ablate KISSAT, not just us.**
S24 closed the "elimination-flag vein" saying depth needed a whole-loop
port. Wrong — one causal probe pair settled it in 10 minutes: at a paired
2.5M-conflict horizon on m29, kissat `--eliminatebound=0` drops to 46%
eliminated = EXACTLY our shipped 47%, props 2.1x, wall +30%;
`--definitions=0` costs only ~5pp. The doubling additional-clauses bound
(`set_next_elimination_bound`) IS kissat's depth. Our own machinery
(SAT_ELIM_BOUND_COMPLETE, built 2026-07-18, default off after the
QG7/Pancake/TT casualties) only needed the RIGHT SCOPE: the band-2 dive
latch, which excludes every July casualty by construction. Second
supporting identity: our props-per-active-var equals kissat's (0.147 v
0.155/conf) — the whole miter props gap was active variables.

**Shipped (9a6807e groundwork + promotion): `SAT_ELIM_BOUND_DIVE2=on`
default.** COMPLETE-round escalation 0→1→2→4→8→16 for formulas with
`restart_dive2_armed_at > 0`. Shipped zero-yield rule stalls at 0-2 on
miters (trace: 8 rounds to 2.2M conflicts, 16k count-bound rejects,
budget never exhausted). With escalation m29 eliminates 1,214→1,669 of
2,575 vars (65%; kissat 73%), props −17%, post-elim literals +43%,
per-prop +22% → wall FLAT at fixed conflicts; the win is trajectory
(fewer conflicts on the collapsed formula).

**Gate (`log/abtest-dive2elim-vs-base-2026-08-21-12-43-45`, 400x2 @
3600 s/16 GB/32 NUMA-balanced): WIN 291 v 288, PASS, PAR-2 975,122 v
985,472, zero contradictions. 285/286 both-solved conflict-IDENTICAL —
the delta is exactly the band.** Mechanism: +dmu28 (UNSAT 2,528 s,
1,072 s margin, FIRST-EVER anywhere; S22 verdict "does not convert even
quiet" now dead), +bwo_bit29 (3,035 s; screen conflicts digit-exact
7,308,747), +sqrt-mitern169 (3,600.0 s at-wall), −m29 (in-band
deterministic reroll 8.59M→10.16M conf; keeps converting standalone
2,269 s ⇒ in-band coin upside on quiet deals). Coins +MVRR +ncc
−bp4_BC012 (127 s margin). PvS_6_6 the only conflict-diff both-solved
cell (+1.47M conf, 412 s, 3,188 s margin). 4 checker-timeouts — the
standing miter proof-size watch class, proofs valid.

**Killed this session (do not re-run):**
- Definitions under bound 16 (SAT_ELIM_DEF + DIVE2): +4 eliminations,
  def_gate_eliminated=0, reject_defcap 8,318 — S24's dead verdict holds
  in the new context too.
- Bound caps 4/8 (SAT_ELIM_BOUND_DIVE2_MAX, knob banked in tree): no
  m29 rescue (max4 2,710 s/9.60M; max8 timeout@4,000 under load) — 16
  is the shape; m29's loss is trajectory reroll, not densification.
- Vivify-throughput on miters: 3x armed budget = 5.4x attempts, 3.6x
  strengthened, conflicts DIGIT-IDENTICAL (10,156,558) — vivify volume
  is trajectory-null on this class; kissat's 179k vivified there is not
  load-bearing. Closes ranked-lever "vivify throughput".
- gdb -p sampling: blocked (ptrace_scope=1); run gdb as parent.

**Band-2 membership truth (SAT_DEBUG_DIVE census, 2026-08-21):** all 12
in-bench 16_16 miters latch; BvP_8_4/9_4/8_6 latch; **BvP_7_6 does NOT
(pre_binfrac 0.283 < 0.30)** — the S15 capture is out of scope by
construction; PvS_6_6 latches (the only solved armed in-band cell).
Trap: the DIVE-CHECK line's second field is post-preprocess binfrac; the
latch reads the THIRD (pre_binfrac). Timeout runs emit no stats JSON.

## SESSION 25 (2026-08-20/21) — dive-scoped trail reuse PROMOTED: gate WIN 292 v 290 (+3/−1, m29 captured IN-BAND); the S16-parked lever revived for the floor-2 latch classes

Speed-round follow-ups measured: pooled binary-implication arena
(WatchPool design, order-identical) = NULL (−0.8% on rbsat; Nested
per-list Vecs are parse-order-localized already; banked default-off as
SAT_BIN_POOL), watch min-cap 16 = null. The WIN: the dive latches
restart every ~30 conflicts, so trail re-propagation dominates
(m29 191 props/conf v kissat 107). SAT_DIVE_REUSE_TRAIL=on (now
default) enables reuse on focused AND stable restarts for latched cells
only — the focused-only form measured a wash; the both-mode form
reproduces the global-reuse trajectories DIGIT-EXACT (m29 8,588,278,
ob19 3,173,688). Standalone: m29 −9-13% wall −5% conflicts, bwo_bit29
3,283 -> 2,069 s, ob_19_4 -> 930-979 s. Gate
log/abtest-reuse-vs-base-2026-08-20-19-30-08: WIN 292 v 290, PASS,
+m29 (in-band, 3,317 s in-gate) +cfi-rigid-t2/+ncc coins −RR_n17 coin,
6 in-band conflict-diff cells only. WATCH: m29/bwo reuse-proofs hit
verify=checker-timeout under gate load (valid proofs, drat budget) —
the standing proof-size watch now covers the miter class.

## SESSION 24 FINAL (2026-08-20) — CLEAN RE-BASELINE 294/400 CONFIRMED with the strongest cell composition ever recorded; elimination-flag vein closed

**The clean single-arm quiet re-baseline
(log/abtest-clean-2026-08-19-21-00-35): 294/400** — matching the
best-ever count with, for the FIRST time, every promoted capability on
one deal simultaneously: ob_19_4 1,396 s (S21 latch), **both S22 miters
in-gate (m29 3,389 s, bwo_bit29 3,283 s)**, manthey 3,248 s (S23 speed
capture), MVRR 3,365 s, RoundRobin_n17 2,999 s, bp4_BC012 3,195 s,
dislog 2,424 s, sqrt169, full TT bank (TT395 129 s / TT496 1,108 s —
bug-free). Head-to-head: ours 294 v kissat 294 same-suite, unique sets
43 v 43; the kissat-only miter family is down from 9 to 7. The 285-289
readings of 08-17/18 are CONFIRMED as bug + queue-pressure artifacts.

Next-session lever prepped: **Flat binary-implication layout**
(BinaryImplications::Flat exists, complete for reads, order-identical =
trajectory-safe, but never constructed in production — Nested
pointer-chase runs everywhere). Needs a hybrid overflow segment to cap
Flat add_edge O(n) inserts for learned binaries. Expected 2-6% on
binary-heavy cells; also SAT_BUMP_SORT_CACHE default-off (unmeasured
recently). Combine with a re-profile after lit_vals.

## SESSION 24 NEGATIVES (2026-08-19) — the elimination-depth flag vein is CLOSED; three dead ends measured and recorded

1. **Band-3 dive latch (myciel6/grs/mod4block): KILLED as gerrymander.**
   Trigger-time shapes are heterogeneous (myciel6 density 3.9, mod4block
   206, grs 2.9; grs pre_binfrac is 0.299 not 0.09) and sted2 (the
   never-perturb cell, 0.677/0.004) sits 0.007-collapse from myciel6
   (0.658/0.011). No clean structural band exists. Do not retry without
   a NEW discriminating axis.
2. **Root gate-aware BVE (SAT_GATE_BVE_SCOPED): ALREADY DEFAULT-ACTIVE**
   on small formulas via profile selection — plain-default runs show
   gate_bve_scoped_adopted=1 (m29 e0=963 -> e1=1000 root elims). The
   87-adopter full-bench scan measured the STATUS QUO, not a candidate.
   Trap: config-struct literal defaults are NOT the shipped defaults;
   check profile overrides before re-measuring any flag.
3. **SAT_ELIM_DEF (kitten semantic definitions in armed rounds): DEAD as
   default.** Full probes: uniqinv40prop still TIMEOUT (the definition
   hammer does NOT crack the SESSION-20 flagship), RoundRobin_n18 still
   TIMEOUT, m29 +5% conflicts (9.51M v 9.04M, no gain), HCP-446 rerolled
   (23.5M v 21.9M, still SAT), bp4_BC012's apparent conversion is its
   known near-wall trajectory (conflicts 8,749,492 digit-identical to
   the speed-A/B gain — not an elim_def effect), and **dislog TIMED OUT
   (fragile-bank kill)**. Default-off re-confirmed with fresh evidence.
4. Elimination-depth conclusion: the m29 57% v kissat 74% gap is NOT
   closable by existing flags (root gates on, armed ext-gates on,
   definitions harmful). kissat's depth comes from its
   eliminate/substitute/vivify whole-loop interleave — the ranked
   sweep-port arc, not a knob. BVE-reject trace confirms 100% of
   rejections are the resolvent-count bound (SAT_TRACE_PREPROCESS_DETAILS
   elim_round lines: reject_count_bound=all, clslim/defcap/budget=0).

## SESSION 23 (2026-08-17/18) — lit_vals per-literal value mirror PROMOTED (~9% engine speedup, trajectories digit-exact); SESSION 22's banked miters CONVERTED IN-GATE (m29 3,260 s, bwo_bit29 3,489 s, both first-ever)

The round's brief was speed/efficiency only. Profile (gdb sampler, m29):
propagation ~60% of leaves with the hot loop already saturated (blocking
literals, inline binary tags, flat watch pool, prefetch, kissat
`searched`). The remaining gap was representational: lit_value()
recomputed sign logic with two branches per call vs kissat's values[]
single load. Change: per-literal mirror (pos/neg slots adjacent via
lit_to_index), maintained at the 4 assignment-mutation sites + rebuild
helper at bulk-overwrite sites (capture_sat_model, lucky trials, test
resets — the debug_assert-on-every-call caught both hidden paths during
development; full 761-test debug suite runs with it active). Unchecked
indexed load (bounds proven by construction).

**Measured:** rbsat probe −8.6% (alternating 3x: 14.60-14.83 →
13.31-13.45 s); m29 paired quiet 2,300 → 2,085 s (−9.4%). **Full-bench
(twin identical arms, log/abtest-speed-vs-speedb-2026-08-17-12-17-43):
twins byte-identical (289 v 289, conflicts 580,784,015 = 580,784,015);
vs the pre-change TSV all 285 shared solved cells conflict-IDENTICAL.
Gains vs yesterday: m29 (3,260 s in-gate, conf digit-exact to the
band-2 probe) and bwo_bit29 (3,489 s) — the SESSION 22 bank cashed —
plus MVRR/sqrt169 coins. The 7 raw cross-day losses ALL failed in BOTH
identical twins (deal-wide drift on a weak deal; TT496 documented
flipper, valves/lockchart/ncc/g2 thin margins, TT395/406 giant-cell
placement lottery) — zero candidate-attributable losses, also true by
construction. Mechanical cross-day gate line reads FAIL (289 v 292);
judged PROMOTABLE under "Judging Trades" and recorded as such.** Commit
7874e01.

Fleet effect: every cell in the 1600-1800 s band gains ~150 s of
margin; the 294-class defaults on a median deal should now read
~295-296.

**SESSION 23 FINAL (2026-08-19) — the "host drift" was mostly a BUG,
now FIXED (feeea27); the corrected same-deal old-vs-new A/B confirms
the engine win.** The first lit_vals build missed growing the mirror in
grow_variables: factor-introduced fresh vars made lit_value read OOB
via get_unchecked (UB) and killed the factoring-heavy SC25 Timetable
class — which is why the 08-17/08-18 twin runs (both the buggy binary)
"lost" TT395/406/496/g2 deal-wide and absolute scores read
292 → 289 → 285. Detection: the same-deal old-vs-new A/B
(log/abtest-new-vs-s22-2026-08-18-13-39-28) showed new losing TT395
which s22 solved in 147 s — impossible for a strictly-faster
identical-trajectory binary; quiet reruns confirmed crashes.

**Corrected A/B (log/abtest-newfix-vs-s22-2026-08-19-03-09-59,
same-deal simultaneous, frozen 2d0d071 snapshot as baseline): 290 v
290 solved, 289 both-solved cells with ZERO conflict diffs, newfix
6.4% faster wall (faster on 211/289), PAR-2 WIN 973,924 v 986,228;
bug-class all recovered and faster (TT395 158 v 160 s, TT406 347 v
354, TT496 1,306 v 1,376, g2 3,195 v 3,408); +MVRR (3,397 s, its
second reproduction) / −ncc (92 s-margin coin).** Across the two
same-deal A/Bs the speed change converted in-gate: MVRR (twice),
RoundRobin_n17 (3,244 s), bp4_BC012 (3,393 s), manthey (3,330 s,
kissat-only FIRST-EVER), plus m29 (3,260 s) and bwo_bit29 (3,489 s) on
the 08-17 comparison — all near-wall cells the old engine cannot reach.

**Measurement rules going forward:** (1) same-deal paired arms are the
only valid comparison (the frozen-snapshot method: build old commit in
a git worktree, place binary in an untracked solver/00-*-snapshot dir
with a no-op build.sh, add a temporary CONFIG_MAP entry — the entry is
NOT committed; recreate on demand). (2) Absolute cross-day counts
remain suspect until a quiet-host re-baseline; the pre-bug paired
lineage stands at the 294-class with the S23 engine strictly faster.

## SESSION 22 (2026-08-16/17) — band-2 dive latch PROMOTED (gate PASS 292 v 291): the 16x16 miter class now runs kissat-parity restarts; miter conversions banked standalone, in-gate blocked only by contention wall

Second application of the SESSION 21 method, on ranked item 1 (the 9
kissat-only 16x16 miters). Mechanism measured on m29
(booth_dadda_origin_and_and_dadda_origin_bit29, kissat 587 s / 8.08M
conflicts / restart interval 43, sweep+congruence+factor all negligible):
pure cadence gap. floor 2 + margin 1.10 converts m29 standalone 2,648 s /
10.1M conflicts (trajectory parity) and booth_wallace_origin_bit29
2,987 s under 13-way load. **Band 2** (in maybe_arm_dive_restarts):
collapse in [0.15,0.35] AND parse binfrac in [0.30,0.50] AND initial
clauses <= 30k — exactly 12 in-bench 16_16 miters + 3 BubbleVsPancake
(all base timeouts) + 5 solved small cells (4 solve at 0 conflicts
pre-trigger; PancakeVsSelection_6_6 arms and improves 2.24M -> 1.70M).
SC25 Timetable excluded by the size cap. No slow-EMA window in band 2
(screen: harmful on miters). **Gate
log/abtest-dive2-vs-base-2026-08-16-13-17-06: WIN 292 v 291, PASS, zero
correctness failures. Honest trade note: the solved +2/−1 (valves +,
bp4_BC012_IXA +, MVRR_n14 −) are ALL out-of-band identical-trajectory
wall coins (MVRR baseline margin 64 s, documented flipper family); the
mechanism content is the PvS tier-2 drop + the banked miter class.**

Measured and rejected this session: walk suppression on band-2 armed
cells (m29 3,157 s / 11.0M vs 2,648 s / 10.1M — the walk's warm phases
guide circuit search); slow-EMA window in band 2. dmu28 (kissat 716 s)
does not convert even quiet — the family ratio (~4x kissat wall) puts
kissat<=700s members at our wall; **the next miter lever is throughput,
not cadence** (m29: 191 props/conf vs kissat 107, 22.7k ticks/conf,
180k live learned clauses on a 2.5k-var formula; kissat eliminates 74%
of vars vs our 57%).

STANDING UPSIDE (no action needed): m29 and bwo_bit29 flip in-gate on
quieter deals (the dislog pattern); ob_24_4/26_4, baseballcover12,
3x BubbleVsPancake are additional in-band rolls.

## SESSION 21 (2026-08-14..16) — dive-restart latch PROMOTED: full-bench 293 → 294/400 (gate PASS, +1/−0, oddball_19_4 FIRST-EVER); the restart-cadence gap vs kissat is now MAPPED and the global form is measured DEAD

**Core finding (mechanism, durable):** our focused-mode glucose-EMA
restart constants are all tamer than kissat 4.0.4 — interval floor 50+log
vs ~1, margin 1.20 vs 1.10, slow-EMA window 4096 vs 100,000. On fat-LBD
counting trajectories (oddball-ttf class: avg LBD 24-32 at level 40-52)
this yields ~460 conflicts/restart where kissat runs ~30, deep dives with
59-lit learned clauses, and 37x per-conflict tick cost — the entire 57x
wall gap on oddball_19_4 (kissat 63 s / 2.75M conflicts). Restart parity
closes the trajectory gap to conflict-parity (3.0-3.5M).

**Global parity is DEAD as a default — measured, do not retry:** the
full-bench A/B (log/abtest-rpmf-vs-base-2026-08-14-16-30-16) LOST 286 v
294 (+8/−16). The gains included real targets (HCP-446, oddball_19/56/67,
lockchart-L190, TT495) but the losses gutted the SAT lottery bank
(bp4/lockchart/fsf/RoundRobin/bivium/VDW/mod2c/TT496). LBD fingerprints
at 100k conflicts show NO clean separation between gained and lost SAT
cells (TT495 63.9 in / TT496 61.1 out; lockchart-g1 83 in / g2 115 out;
oddball_56 137 in / _80 128 out) — pure trajectory lottery. Early-window
(100k-conflict) LBD also fails as a discriminator (ob_19_4 measures 20.6
there; the fat signature develops by ~1M).

**What PROMOTED (commit chain e8676d4 → 18aa624 → this): the structural
dive latch, SAT_RESTART_DIVE=on by default.** One-shot check after root
preprocessing: non-binary clause-mass collapse >= 0.77 AND parse-time
binary fraction in [0.50, 0.85]. Trigger-time truth (SAT_DEBUG_DIVE=on):
trio at collapse 0.782-0.834 / binfrac 0.708-0.718; nearest non-members
oddball_80 tto 0.745/0.986, ER_400 0.543/0.987, MVRR 0.308/0.996 — the
SAT-lottery families all carry binfrac >= 0.96 and are excluded by the
ceiling. Full-bench shape-scan (scripted, 400 cells): EXACTLY 10 in band
= 3 target timeouts (ob_19/24/26_4) + baseballcover12 (kissat-unsolved
too) + 3 SAT-at-0-conflicts cells + linked_list (6k conf) + ttf siblings
13_5/17_5. Latch = floor 2 + margin 1.10 + slow window 100k + kissat-style
bias-corrected EMA warmup (alpha_eff = max(alpha, 1/(n+1)), latch-only;
without warmup the pinned slow EMA thrashes: 6.9M vs 3.18M conflicts on
ob_19_4). **Gate (log/abtest-dive-vs-base-2026-08-15-12-18-53): WIN
294 v 293, +1/−0, promotion_gate=PASS, zero correctness failures, 396/400
cells conflict-identical; oddball_19_4 first-ever UNSAT in-gate (3.18M
conflicts), 13_5 improves 1.84M→1.69M, linked_list 6004→3248, 17_5
119k→826k (still 16-24 s, priced).**

Recorded negatives (do not repeat):
- ob_26_4 converts ONLY with latch + unarmed walk-min (2,174 s once);
  floor+margin and full-parity latches both time out even quiet. Walk-min
  as default endangers the walk bank — left out. It remains in-band
  upside on lucky deals.
- ob_24_4 never converted under any variant (kissat 789 s).
- HCP-446 walk-effort bracket (SAT_WALK_EFFORT_YIELD_ARMED, inert knob
  banked): shipped yield-armed effort 50 is already optimal — 1 fails,
  100 = 3,060 s, 250 = timeout vs 2,676 s at 50. The HCP conversion lever
  is NOT walk effort; it remains contention-margin (lower-contention
  scheduling or a faster collapse).
- myciel6 (12.0 LBD) and grs-32-128 (level 483) sit OUTSIDE the band and
  converted only under the global-parity env (standalone 2,690-3,391 s);
  candidates for a second, different discriminator if one exists — do NOT
  widen this band to chase them (VDW at 22.0/33.1 is adjacent).

Free riders in tree (inert): SAT_RESTART_FLOOR / SAT_RESTART_MARGIN
(global restart knobs, defaults unchanged), SAT_WALK_EFFORT_YIELD_ARMED,
SAT_DEBUG_DIVE + SAT_RESTART_DIVE_COLLAPSE/BINFRAC tuning knobs.
Validation: 761+5 tests, smoke 9/9, rbsat 100001/196258/17,758,017
digit-exact dive on AND off, no-fire verified on VDW/MVRR/TT496.

## SESSION 20 FINAL VERDICT (2026-08-13) — yield-latch arc closed as a BENCH-WASH after two full A/Bs and per-cell calibration; two standalone first-evers banked as evidence

The complete arc (all default-off in tree, commits 9b78fa8..978cbd6+):
latch + aggressive cadence + wide envs + kitten flips + repr streaming +
fast kitten + calibrated band (abs >= 1000 equivs) + early probe
(SAT_SWEEP_YIELD_PROBE, declines byte-identically).

**Measured outcomes:** STANDALONE conversions of two kissat-only cells —
HCP-446-105 (SAT 2730 s, model independently verified vs all 247,657
clauses; formula collapsed 51% by the cascade) and dislog_a14 (SAT
~2400-2500 s, reproduced in-gate in BOTH A/B deals). But the FULL-BENCH
A/Bs: 20-permille band LOSE 290 v 295
(log/abtest-cand-vs-base-2026-08-12-18-25-30: armed too widely);
calibrated band LOSE 292 v 293
(log/abtest-cand-vs-base-2026-08-13-07-58-47: dislog + bp4 gained, but
HCP cannot beat the in-gate wall — 2730 s standalone + 32-way contention
> 3600 — and sqrt169/oddball-class collateral persists). HCP's yield
develops too late for the early probe (103 equivs at 150k conflicts v
1490 at 810k). Tightening the band further would select exactly dislog =
one-cell overfit, forbidden. **VERDICT REVISED (SESSION 20g, same day): PROMOTED after the
non-arming verification.** The three calibrated-A/B losses were each
PROVEN non-arming (sqrt169 probe = 7 equivs v the 1000 floor;
oddball_19_4 and reconf10 zero ARMED lines through 3M conflicts) —
byte-identical AND wall-identical trajectories in the cand arm, i.e.
pure contention coins by construction, all three documented flippers.
Under "Judging Trades" (N=3 coins with written justification v
mechanism-validated capability) the trade PROMOTES:
SAT_SWEEP_YIELD_ESCALATE=20 + SAT_SWEEP_YIELD_MIN_EQUIVS=1000 default
ON (probe stays off). dislog_a14 (kissat-only) is the durable capture —
1680 s at the shipped default, in-gate both A/B deals; HCP-446 remains
a standalone-only capability (2676-2730 s, wall-borderline in-gate) and
16_2-class collapses are upside. Fingerprints digit-exact under the
default. Next-session notes unchanged: whole-loop sweep port (uniqinv40
acceptance) and lower-contention scheduling would both convert HCP.

## SESSION 20 (2026-08-12) — NO PROMOTION: the uniqinv40/sweep-equivalence arc mapped to its root; miter-congruence definitively killed; yield-escalate latch banked default-off

**Flagship target: uniqinv40prop (kissat 51 s UNSAT, we timeout — a 70x
structural gap).** kissat's measured recipe there: 3,799 sweep equivalences
(30% of vars) over 24 sweeps / 130k kitten solves + 3,108 congruence
matches, then 549k conflicts. Layer-by-layer findings (all measured, none
speculative):

1. **Congruence matching is NOT the entry point:** every one of our 12,092
   extracted AND gates has a DISTINCT input pair — 0 syntactic merges exist
   pristine. kissat's 261 initial congruent vars only arise after its
   substitute→re-extract cascade reaches critical mass. (Also killed for
   the miter class: pristine boothdadda29 extraction = 5,162 gates but 1
   merge — booth/dadda halves share no syntactic gate structure; the plan's
   'congruence blind on miters' hypothesis is DEAD, and the stats-only-
   on-apply artifact that suggested it is noted below.)
2. **SAT_SWEEP_YIELD_ESCALATE latch built (default OFF, commit 9b78fa8):**
   percent-scale equivalence yield latches retire-scan + escalation +
   seed budget 2048 + substitution + aggressive cadence. On uniqinv40 it
   arms at round 1 (375 equivs) and substitutes ~113 distinct vars — then
   the cascade STALLS (~500 distinct equivalences total vs kissat 3,799).
3. **Environment size is NOT the residual:** depth-8/8192-var environments
   yield ZERO (the 2000-solve budget dilutes; pairs are LOCAL).
4. **Transitive pair-waste is NOT the residual:** a union-find skip
   (prove_facts_budgeted_opts, yield-armed rounds only) changed nothing —
   yields identical. The residual is (a) duplicate proving across
   OVERLAPPING environments (whole-env retirement exists only in the tick
   engine) and, deeper, (b) kissat's per-sweep candidate mechanics
   sustaining high yield across 24 sweeps where ours dries up after 2.
   **Closing this needs a faithful kissat sweep.c pair-mechanics port — a
   full session, promoted to ranked item 1.**
5. **SESSION 20b continuation (same day): kitten `flip_literal` PORTED
   (kitten.rs, kissat parity: rewatch-or-fail walk of the true literal's
   watch list; free model-space disproof of backbone/equivalence
   candidates) and wired into the yield-armed sweep (flip pre-tests before
   every solve). Wide-env armed bounds folded into the latch (4096 vars /
   16384 clauses / depth 5 / 64 seeds — probe: round-1 yield 375 → 704 and
   the cascade SUSTAINS ~50/round instead of dying). uniqinv40 still does
   NOT convert at 3600 s (~10x short of kissat's 3,799-equivalence
   critical mass). THE REMAINING PORT CHUNK, precisely: (a) kissat
   `sweep_repr` — substitute proven representatives INSIDE the kitten
   environment mid-sweep so the region collapses while being swept (ours
   applies equivalences only after the round via ELS); (b) kitten solve
   throughput (kissat ~18 µs/solve; profile ours). Acceptance test
   unchanged: uniqinv40 at 3600 s. All SESSION 20/20b knobs default-off;
   defaults byte-identical to SESSION 19.**

par32-2/dubois50 XOR recovery also closed this session (par32's pure-XOR
subsystem is consistent — SAT_GAUSS_MIN_COVERAGE env banked; dubois50's
clause var-sets are all distinct post-transformation). Validation: 761+5
tests, smoke 9/9, rbsat fingerprint digit-exact (all new knobs default
off; defaults byte-identical to the SESSION 19 promotion).

## SESSION 19 (2026-08-11/12) — frontier-sweep counting engine PROMOTED: mchess_20 FIRST-EVER (0.011 s refute, drat-trim VERIFIED); ranked research arc 3 delivered

**Shipped (commit 09a271b + promotion): `src/sweepcount.rs` + SAT_SWEEPCOUNT
default ON.** Pre-search refutation of exactly-one bipartite cover imbalance
(mutilated-chessboard class). The proof design that unblocked the arc: NOT
the H^4 inductive php closer (1.15G lines at H=198, RAT-scan-dead) but a
FRONTIER SWEEP — order cells by bandwidth, keep banded unary counters over
the open-edge frontier (width 21 for mchess_20, not H=198!), advance the
invariant FB−FW=δ per cell via single-pass-RUP lemma batteries, empty
clause when the frontier sweeps out with δ=2. 291k lines, 2.5 MB, verify
115.7 s. The battery engineering (all validated by drat-trim forward
checking on synthetic 4x4/8x8/20x20): definitions RAT-pivot-first; extend
E1-E3 + reverse H0/H1/REV + level-monotone M on the append side; bridge
D1-D5 + per-removed-edge transfer T on the removal side; two-direction
banded invariant on top. KEY LEMMA-ENGINEERING LAWS learned (for the next
proof engine): (1) a lemma is single-pass-RUP only if every case branch is
resolved by an EARLIER lemma — emit per-edge helper batteries BEFORE their
OR-lifted forms; (2) negation of a constant-false counter level is
constant-TRUE (vacuous lemma), never "drop the literal" — conflating these
emits false claims; (3) band saturation needs a 4-state level type
(true/false/var/UNTRACKED) — untracked levels must skip the lemma entirely.

**A/B `log/abtest-cand-vs-base-2026-08-11-19-37-55`:** +mchess_20 (UNSAT
0.05 s in-gate, proof verified ok); ALL 291 shared solved cells
conflict-IDENTICAL — the decline-is-identity claim proven at bench scale;
raw 293 v 294 solely from two documented thin-margin flippers
(valves-gates 33 s, oddball_19_4 103 s) swapping on wall under contention.
Judged per the trade rule: 2 wall coins (test 1, ≤120 s margins, identical
conflicts) v a deterministic first-ever — PROMOTED. Zero contradictions,
zero correctness failures.

**Also measured this session (negatives, recorded):** par32-2's pure-XOR
subsystem is CONSISTENT (gauss's coverage decline at 0.798 was honest —
SAT_GAUSS_MIN_COVERAGE env added, default unchanged); dubois50's clauses
all sit on DISTINCT var sets (transformed instance — no XOR groups to
extract; both stay both-timeout). rook-51/52/56 do NOT fit sweepcount
(P==H balanced rook constraints; their hardness is not color imbalance).

## HEAD-TO-HEAD RE-BASELINE (2026-08-10, user-requested double-check) — solver12 292 v kissat 294 same-host same-deal; gap −2; NUMA-balanced pinning landed

Sequential full-bench runs, 3600 s / 16 GB / 32 NUMA-balanced cores (no
contention between arms):

| solver | solved | PAR-2 | unique | TSV |
|---|:--:|--:|:--:|---|
| solver12 (promoted defaults) | 292/400 | 944,307 | 42 | `log/abtest-solver12-2026-08-10-00-01-22/solver12/results.tsv` |
| kissat 4.0.4 | 294/400 | 930,904 | 44 | `log/kissat-full-20260810-073149/results.csv` |

- **solver12 REPRODUCED its promoted 292 exactly** (verify 288 ok / 4
  checker-timeout / 0 fail — the promotion deal's 7-timeout scare did not
  recur). **kissat scored 294 v its recorded 296** (its own ±2 deal
  variance). Use THIS pair as the reference gap (−2) for same-host
  same-deal comparisons; the 07-29 kissat 296 run predates the balanced
  pinning and is a different deal.
- Unique-set shapes: solver12's 42 = engineered capabilities (php/counting
  x11, RoundRobin/MVRR gate-BVE x10 incl. the walk-giveup first-ever
  n17_d15, oddball_tto_zp x6, xor/tseitin x3, VdW x2, walk-era gains).
  kissat's 44 = 16x16 miters x9 (boothbit29/boothdadda29 flipped BACK to
  kissat this deal — wall-margin swing cells), starved BMC x7, pj giants
  x2, lottery tail. Both-timeout 64.
- **Tooling (commit 2cf3aec): NUMA-balanced worker pinning in
  feature_ablation.py (`numa_balanced_cores`) + run_kissat_full.sh
  (CORE_ORDER_STR, offset = window shift).** Old `range(jobs)` put 18/32
  workers on socket 0; new order alternates sockets over physical cpus
  (16+16 at 32 jobs), SMT spill only past 36. Verified live (taskset
  affinities one-per-socket; order recorded in kissat meta.txt).

## SESSION 18 (2026-08-08/09) — adaptive walk giveup PROMOTED: full-bench 291 → 292/400 (gate PASS, +1, a both-timeout first-ever); the walk vein is now CLOSED and the miter/near-miss levers are mapped exhausted

**Promoted: `SAT_WALK_STALL_GIVEUP=16`.** Walk cannot refute UNSAT; the
latch class mixes SAT walk-targets with UNSAT near-misses. Giveup abandons
walking once the best walk min-unsat stalls K=16 walks (RATE-based: must drop
≥1/64 to count as progress — marginal UNSAT creep counts as a stall),
returning the budget to CDCL. Byte-identical on SAT cells by construction.
A/B `log/abtest-cand-vs-base-2026-08-09-06-42-44` (gate PASS, zero
correctness failures, no SAT regressions): **292 v 291; +RoundRobin_n17_d15
(FIRST-EVER, both-timeout, kissat can't either) +mod2c; −RoundRobin_n18_d15
(same-family 355 s thin-margin wall swap).** Modest (+1, noise-band-adjacent)
but the gain is a deterministic first-ever and the mechanism is safe. Gap to
kissat now −4.

**THE EXHAUSTION MAP (this session's real deliverable — do not re-run these):**
- **Miter family (9 cells, biggest gap): SATURATED for flags.** Mid-search
  PROBE finds 0 units (23,480 attempts); BACKBONE 0; gate-BVE already on;
  vivify volume already at kissat parity (182k attempts) via deduce. Residual
  is pure CDCL trajectory quality (kissat refutes in 6M conflicts, we need
  >20M) — needs a decision/learning-quality mechanism, not a pass.
- **RoundRobin/near-miss via ELIM-ARMING: DANGEROUS, closed.** Forcing
  elim-yield arming (SAT_ELIM_PRODUCTIVE_MIN_PCT=10) on RoundRobin caused an
  UNBOUNDED non-CDCL runaway — probes ran ~14 h with SAT_LIMIT_WALL_SEC never
  firing (wall limit is CDCL-loop-only). Confirms the 2026-07-14 lottery +
  runaway warning; do not re-open without an elimination bound.
- **Walk latch 1M vs 500k: 500k CONFIRMED optimal.** A biased-subset screen
  favored 1M (14/19) but the full bench LOST 286 v 291 — the classic
  screen-doesn't-transfer trap. 500k stays.
- **tseitin_grid: research-scale.** The tseitin engine detects the full
  62,500-node grid component but proved=false — refuting 2D grid cycle
  structure is a proof-engine extension, with checker-cost risk (grid_n400
  already closed under the RAT-scan law).

## SESSION 17 (2026-08-06/07) — walk-latch second wave PROMOTED: full-bench 285 → 290/400 (gate PASS, +11/−6); gap to kissat −6; rbsat walk-solved

**Promoted defaults: `SAT_WALK_WARMUP_UNARMED=on` (new knob — kissat
warmup.c, scoped to never-armed walkers; the 2026-07-17 warmup NEGATIVE was
measured entirely on ARMED walkers, which stay byte-identical) +
`SAT_REPHASE_UNARMED_MIN` 1M → 500k (earlier latch = more walk runway).**

Full-bench A/B `log/abtest-cand-vs-base-2026-08-07-01-51-08` (gate PASS,
zero contradictions/correctness failures): **290 v 285. Gained 11:
ITC2021_Early_12 (834 s; solves in all 4 measured deals/arms since the
latch) + bp4_BC012_CSO_FPBEQ (both former kissat-only);
VanDerWaerden_pd_2-3-27_663 + lockchart-group2 x2 (FIRST-EVERS — nobody
solved these at 3600 s); rbsat-v1375 (the flagship wall-coin flipper of the
whole project, now WALK-SOLVED at ~7.5M conflicts in 4 consecutive
deals/arms — no longer a coin); reconf10 + frb80 (the 16b reroll losses
recovered); sum_of_3_cubes, valves-gates, oddball_57. Lost 6 walk-lottery
classmates (ER_400.apx_2, vmpc_28, oddball_56, bp4_IXA_LPI, mod2c,
oddball_19_4 — every one a documented member of the deep-unarmed rebalance
class; class-level net across 16b+17 = +9). PAR-2 955,537 v 993,612;
tier-2 conflicts flat. Checker-timeouts 3→7 — all big-proof UNSAT solves,
drat-trim BUDGET (none rejected); caveat class, watch it.**

Screen `log/abtest-warm-vs-thresh-vs-warmthresh-vs-base-2026-08-06-23-35-05`
(16 cells): warmthresh 12/16 v base 9/16 with each mechanism confirmed
alone (warm recovered frb80+VdW-23-accel; thresh captured ITC_Early_12 at
408 s + case6). dislog is NOT a latch target (it ARMS and already walks
4.3G steps — its gap is elsewhere). ITC_Late_10 still stands (walks but
does not convert). Validation: 756+5 tests, smoke 9/9, rbsat/MVRR
fingerprints digit-exact (both below the 500k latch).

## SESSION 16b (2026-08-06) — deep-unarmed rephase/walk latch PROMOTED: full-bench 281 → 286/400 (gate PASS, +9/−4, tier-2 −81.8M); SEVEN former kissat-only cells captured

**The discovery:** never-armed formulas structurally could not rephase or
walk — `config.rephase` defaults off and ONLY the arming/endgame paths set
`rephase_enabled`, so the walk-scale SAT class ran ZERO walk steps at any
depth (ITC_Early_12 / ITC_Late_10 / ER_400.apx_1 measured `rephases=0,
walk_steps=0` at 1.2M conflicts while kissat walks 100-360M steps there).
Corollary: `SAT_WALK_EFFORT_UNARMED=200` (promoted 14d) was DEAD CODE —
every rephase-enabled cell is `inprocess_aggressive`, so the unarmed branch
never executed anywhere.

**The promoted shape (commit after d6ea413):**
`SAT_REPHASE_UNARMED_MIN=1_000_000` default ON — enable the kissat-parity
rephase/walk cycle once a never-armed formula reaches 1M conflicts (the
endgame philosophy: perturb only losing trajectories; every unarmed cell
finishing below 1M is byte-identical BY CONSTRUCTION — rbsat
100001/196258/17,758,017 and MVRR 267,199 digit-exact) — plus
`SAT_WALK_EFFORT_UNARMED` default 200 → **50** (kissat walkeffort parity;
the screen measured 200 OVERWALKING: e50 9/14 v e200 6/14 v base 6/14 —
e200 lost vmpc/mod2c/sted2 that e50 wins).

**Full-bench A/B `log/abtest-cand-vs-base-2026-08-06-03-28-37`** (400x2
@3600 s, gate PASS, zero contradictions/correctness failures,
checker-timeouts 5→4): **cand 286 v base 281. Gained 9 (all SAT, all the
deep-unarmed class): ER_400_20_7.apx_1, sted2_0x0_n219, mod2c-rand3bip,
case8, fsf-300-354 x2 — all six former KISSAT-ONLY — plus 170223547
(walk-solves in 51 s right at the latch, was a coin timeout), bp4_BC012_AM,
mp1-Nb7T45. Lost 4: bp4_TCO (184 s, the documented deal coin), VdW-23
(walk-reroll — solved in the screen deal at 3358 s), reconf10_22 + frb80
(reroll losses inside the allowance). Tier-2 conflicts −81.8M across 47
changed both-solved cells; PAR-2 987,867 v 1,028,679.**

Screen (`log/abtest-e200-vs-e50-vs-base-2026-08-06-01-37-34`, suite
`benchmarks/unarmedwalk-2026-08-06`: 5 walk targets + 9 deep-unarmed
coin-class canaries): e50 9/14 v base 6/14, zero losses. ITC x2 and dislog
did NOT fall (still kissat-only) — the latch walks them now but they need
more than phase luck. Validation: 756+5 tests, smoke 9/9.

## SESSION 16 (2026-08-04/06) — NO PROMOTION: the late-armed re-screen space is now mapped; trail reuse PARKED after full evidence; five arms closed with data

**Verdict: defaults unchanged (identity fingerprints digit-exact all
session). The full-bench baseline stays 279/400 promoted; same-config deals
this week scored 276/279/280/281 — the ±2-4 variance calibration holds.**

What was measured (all screens on `benchmarks/miterded-2026-08-02` or
`benchmarks/reusefocused-2026-08-06`, full A/B on sat-comp-2025 400x2):

1. **Profile (gdb SIGINT sampler, boothdadda29 @2.5M conflicts): ~72% of
   wall is `propagate_impl`**; walk negligible; analysis ~14%. Wall/prop is
   only ~1.2x kissat (654 v 537 ns) — the earlier 49-v-26 "ticks/prop" read
   overstated (different accounting units). The real gap is props/conflict
   (194 v 108), dominated by restart re-descent (16,194 restarts / 2.5M
   conflicts, zero reuse) and DB/trajectory quality. SAT_WATCH_POOL and
   SAT_WATCH_INLINE_BIN are ALREADY default-on (stale doc comments say off).
2. **Banded vivify-sort and banded tier3: CLOSED** (screen
   `log/abtest-reuse-vs-sort-vs-tier3-vs-base-2026-08-05-00-20-53`: 7/23
   each v base 8/23 — rerolls without gains, even inside the 500k band with
   deduce active).
3. **Trail reuse (kissat restartreusetrail): PARKED with full evidence.**
   Wiring gap found+fixed (the miters arm via the VIVIFY-YIELD path,
   congruence_merges=0 — the knob only wired through the congruence path;
   commit a726262). Once live: screen WIN 9/23 v 8/23 (boothdadda29
   FIRST-EVER, every UNSAT miter −10-15% conflicts, canaries exact) but the
   full A/B (`log/abtest-cand-vs-base-2026-08-05-08-46-46`) LOST 280 v 281
   with tier-2 +10.9M: the SAME determinism that wins the UNSAT miters
   (boothdadda29 8,759,563 conflicts EXACT across two deals) deterministically
   REROLLS late-armed SAT cells (Circuit_multiplier24 — stable 4,992,637-conf
   trajectory in two deals — and DLTM_twitter774, both fat-margin losses;
   oddball_ttf/ER_400 +2-11M conflicts). The =focused variant (96% of miter
   reuse events are focused-mode) does NOT separate them
   (`log/abtest-focused-vs-both-vs-base-2026-08-05-22-30-51`): Circuit24
   still dies, boothdadda29's gain NEEDS stable-mode reuse, and the miters
   land between base and both. **Law: reuse's per-cell effect is
   deterministic but its sign is per-cell — there is no runtime discriminator
   separating late-armed UNSAT grinders from late-armed SAT-capable cells.
   Shipping it trades ~2 stable SAT cells for ~1 first-ever miter. Knobs
   banked: SAT_RESTART_REUSE_TRAIL_ARMED=on|focused (+_MIN band), both
   paths wired.** The aggressive cadence bundle (floor=1, margin=1.10 +
   reuse) is CLOSED outright (7/23).
4. **Ranked-item hygiene:** SWEEP_SUBST percent-mass (old item 3) PRUNED —
   SESSION 14c already measured SAT_SWEEP_SUBST=on flipping 0/6 on
   miters+uniqinv at 3600 s idle; a safety threshold cannot rescue a
   mechanism that does not fire on its target. mchess_20/rook decode
   (below) moved to a research arc.
5. **mchess_20 decoded (760 domino vars, pairwise AMO, 398 exactly-once
   cells): it IS the direct-php shape** — 200 var-disjoint black-cell covers
   v 198 white-cell AMO holes — but the counting core is PHP(200,198) and
   the inductive closer is ~3/4·H^4 ≈ 1.15G proof lines at H=198:
   infeasible. The family (mchess_20, rook-51/52/56, all nobody-solves
   except rook-51=kissat-only) needs a CARDINALITY-STYLE proof engine
   (totalizer/pseudo-Boolean simulation in DRAT) — a genuine research arc;
   naive totalizer LB/UB groupings do not compose in RUP (the LB needs the
   injective-mapping argument = php again). Park until someone designs the
   proof shape on paper first.

## SESSION 15 (2026-08-02/04) — banded vivify-deduce PROMOTED: full-bench 276 → 279/400 (gate PASS, A/B WIN +5/−2); backbone.c port landed and measured a no-op (free rider, default off)

Full-bench A/B `log/abtest-cand-vs-base-2026-08-03-10-13-35` (400x2 @3600 s
/16 GB/32 cores, simultaneous start, proofs verified, gate PASS, zero
contradictions / zero correctness failures):

| arm | solved | conf(own solved) | PAR-2 |
|---|:--:|--:|--:|
| cand (`SAT_VIVIFY_DEDUCE=on`, banded) | **279/400** | 532.9M | 1,041,267 |
| base (SESSION 14d defaults) | 276/400 | 554.7M | 1,057,324 |

**Gained (+5):** Circuit_multiplier24 (SAT 1917 s, FAT margin — a named
walk-scale gap cell), BubbleVsPancakeSort_7_6 (UNSAT 2274 s, FAT margin — gap
family), valves-gates + bp4_TCO_IXA_FPBLE_ZR + bp4_BC012_IXA_LPI (banked cells
base dropped this deal; retained/recovered). **Lost (−2):**
MVRoundRobin_n14_d10_v2 (base margin 82 s = thin wall coin) and
sum_of_3_cubes_37_bits_87 (REAL SAT reroll: base solved at its stable
894,247-conflict trajectory — identical in 3 prior deals — while deduce
changed cand's deal; expect it to flip back some deals). Tier-2: −14.7M
conflicts across the 37 changed both-solved cells; the mechanism cells all
shortened 10-30% (sqrt-mitern169 −1.43M, lec_mult −1.10M, boothbit29 −0.96M,
oddball_19 −3.94M, PancakeVsSelection_6_8 −3.61M, ER_400 −3.28M; worst
regression case11 +5.0M, still solved).

**What shipped (commits a1bbb5f, 2549801, + the promotion commit):**

1. **`SAT_VIVIFY_DEDUCE` default ON, banded** (the promotion). The kissat
   `vivify_deduce` reason-cone mechanism was built 2026-07-15 and shelved
   after the UNBANDED armed screen lost on EARLY armers (ibm +133% conflicts,
   oski20 +146 s). SESSION 15 added `SAT_VIVIFY_DEDUCE_ARMED_MIN=500_000`
   (the SESSION 14d reduce-law arming-time discriminator): deduce fires only
   where `inprocess_armed_at_conflict >= 500k`, so TT/oski/vex/oddball-class
   banked early armers are byte-identical BY CONSTRUCTION (miterded screen:
   all five canaries conflict-EXACT; identity refs digit-exact). Mechanism:
   boothdadda29 probe @2.5M conflicts — vivify hit rate 14.8% → 28.5%
   (kissat 34%), strengthened 27,491 → 53,823, wall 318 → 311 s.
2. **`src/backbone.rs` — full kissat backbone.c port, default OFF.**
   Stacked-probe failed-literal rounds over a private binary-implication-graph
   propagator, BIG-UIP analysis, RUP units through the learn_lucky path,
   kissat-parity flags/rounds/2%-effort. Tier-1 on the miter class: **ZERO
   units found — and kissat's own backbone finds 2 units there** (its 341k
   backbone ticks are cadence, not content). This re-confirms the 2026-07-15
   "killed without building" verdict buried in commit 038f9c1 — the ranked
   backbone item in earlier plan revisions was STALE. The pass is a
   zero-mutation zero-cost rider (bb arm conflict-identical to base on all
   23 screen cells): keep OFF; only re-arm if a family with a RICH binary
   implication graph (large edge count + failed-literal yield) shows up.
3. **Tier decomposition that found the real lever (boothdadda29, identical
   2.5M-conflict horizon):** solver12 318 s / 23.9G search ticks vs kissat
   145 s / 6.97G — 3.4x ticks (49 v 26 ticks/prop AND 194 v 108
   props/conflict) with kissat vivifying 6.5x more clauses (179,349 v
   27,491) and walking only 0.12% of wall. Deduce closes part of the
   hit-rate hole; the residual rate gap (still ~2x wall on miters) is the
   #1 remaining mechanism target.

Screens: miterded 4-arm (`log/abtest-ded-vs-bbded-vs-bb-vs-base-2026-08-02-
17-45-21`, 23 cells @3600 s): ded 8/23 v base 7/23 (gained sqrt-mitern169;
boothbit29 8.97M → 8.01M conf), bb ≡ base conflict-exact, bbded ≡ ded
conflict-exact (no antagonism, no backbone contribution). New suite:
`benchmarks/miterded-2026-08-02` (23 cells = miterarmed-2026-08-01 + sqrt169
+ lec_mult + boothdadda28/29 + mult16_22). Validation: 756+5 tests (+13 this
session), smoke 9/9, rbsat 100001/196258/17,758,017 + MVRR 267,199
digit-exact both flag states.

## SESSIONS 14b/14c/14d (2026-07-29..08-02) — pruned summaries

- **14d (280/400, +4/−0):** banded `SAT_REDUCE_FRACTION_ARMED` (+ `_MIN=500k`
  arming-time band — the discriminator SESSION 15 reused) un-blinded the
  reduce law on late-armed miters: FIRST-EVER 16x16 miter solve (boothbit29),
  + sqrt-mitern169/lec_mult/shuffling-1. Also `SAT_REPHASE_ARMED_ONLY=off` +
  `SAT_WALK_EFFORT_UNARMED=200`. Full text: rev 93ab682.
- **14c (277/400, +6/−0):** php-detector coverage — inductive PHP proof
  engine (Cook's ER reduction, ~H^4 lines v factorial), direct-php detection,
  AMO-connectivity partition voting, parse-time structure stash: 5 first-ever
  both-timeout hard-core cells (cliquecoloring/clqcl/fphp/rphp). Full text:
  rev d838757.
- **14b (271/400, +10/−4):** three runaway-pass bugs fixed (sweep-kitten
  unlimited budget, gauss ordering spin + 31 GB fill-in, mid-giant BVE 8 GiB
  arena doubling) + `SAT_REDUCE_FRACTION` default ON + thresholded `SAT_ELS`
  ON + root-pass scoping law (percent-mass decline-is-identity gates are the
  ONLY shippable root-pass shape). Full text: rev 416adae.

## RANKED PLAN (2026-08-28, post-SESSION 29)

SESSIONS 15-28 took the bench 279 → 297/298-class. The four productive
shapes stand: NEW ENGINES (sweepcount), SCOPED-PARITY LATCHES
(dive-restart, dive2-elim, chrono-strict-auto), CAUSAL KISSAT ABLATION
(grid first, build second), and WALL-BAND PROFILING (S27b) + now
REFERENCE-MODEL AUDITING (S29). S29 closed ranked item 1 as an ENGINE
(faithful sweep built, correct, capabilities proven) but NOT as a
default (no runtime discriminator — see the S29 section). Next leads:

1. **A discriminator for SAT_SWEEP_FAITHFUL (the highest-leverage open
   question).** The engine converts uniqinv40 (any scope) and b18
   (global only) and cuts armed-class conflicts 30-60%, but every
   tried scope forfeits walk-bank capability. Measured-dead axes:
   per-round yield, cumulative equivs, walk_steps, congruence merges,
   dive2 latch (S29 section — do NOT re-gate variants on these).
   Untried axes with mechanism content: (a) decline-is-identity FIRST
   CALL — prove-only probe pass, commit the engine only if its OWN
   first-call yield crosses a floor (S14b root-pass law shape; needs
   per-call yield data: winners' first calls vs casualties'); (b) a
   post-substitution REVERSAL bound — cap fsweep to N rewrites until
   productivity confirms; (c) UNSAT-refutation-progress signals
   (learned-clause LBD trend under fsweep vs not). Budget probes
   BEFORE any gate.
2. **b18-class root sweep (global-mode-only capability).** b18 needs
   the preprocess-time faithful sweep (armed misses it at 3600 s). A
   ROOT-ONLY scope (sweep once at preprocess, never mid-search) is
   unmeasured as its own arm — it may keep most of the walk bank
   (mid-search rerolls are the bigger casualty channel) while keeping
   b18 + part of uniqinv40's head start. One screen answers it.
3. **Scope discriminator for the giant BMC step class (nla-dijkstra/
   pj2016).** Unchanged from S28: measured strict upside, ZERO
   congruence merges, auto scope misses them.
4. **Band-scoped escalation reuse (S26 shape, unchanged).** COMPLETE_ALL
   knob in tree; needs a NEW structural discriminator for b18/grs/ncc.
5. **Miter family residual (4-6 kissat-only mapped miters).** All
   single-mechanism levers closed (S26/S27/S28); fsweep on the miter
   band is a measured in-gate LOSER (m29/sqrt169 rerolls) despite
   standalone m29 2,386 s — composite throughput remains the story.
6. **Medium-1800 re-baseline (bookkeeping, OVERDUE).**
7. **Checker-timeout proof-size watch (standing, miter class).**
8. **PARKED/CLOSED: strict-chrono x armed-restart coupling (S28b,
   measured dead); sweepcount generalization; walk vein;
   starved-BMC/XOR; factor.c (in tree); unscoped escalation (S27);
   UNSCOPED strict chrono (S28); definitions (S24+S26); vivify volume
   on miters (S26); flywheel for BMC (S27); sweep-faithful scope
   variants on the four dead axes (S29).**

## Current state

- **2026-HOLDOUT GENERALIZATION TEST (2026-08-29/30, THE overfit
  answer — read this before planning any new band): solver12 160/400
  PAR-2 1,841,674 v kissat 197/400 PAR-2 1,575,285 on the SAT
  Competition 2026 main track (never seen by the tuning loop; only
  8/400 hashes overlap 2025).** Zero contradictions; 159 proofs
  verified ok + 1 checker-timeout. Runs:
  `log/abtest-solver12-2026-08-29-10-22-36` v
  `log/kissat-full-20260829-202806`; suite downloaded to
  `benchmarks/sat-comp-2026/` (gitignored, MANIFEST.txt tracked).
  **The 2025 lead (+4) does NOT generalize: −37 out of distribution.**
  Decomposition: we-only 15 = 6 engine solves (php detector fired on
  the BRAND-NEW php_sudoku family + new clqcl sizes + rphp/xor — the
  engines ARE general detectors, their families are just rarer in
  2026) + 9 search wins (incl. Circuit_multiplier36 — family transfer
  from the 24-variant). kissat-only 52 = broad NEW families we have no
  machinery for (schooltt x6, lightsout x5, cabp x5, SDP x3,
  polyomino-pair x6, hitag2 x2, adv_gc, stable, atco, ER_500) PLUS the
  standing miter/BvP gap on fresh members (sqrt-mitern168,
  booth_dadda_mapped bit28-class, BvP_7_7/8_5). Both-solved 145:
  kissat wall-faster on 112 (77% — same shared-cell ratio as 2025).
  **Law: our 296-class number is a 2025-DISTRIBUTION number. The
  engineered families (RoundRobin/MVRR, oddball tto_zp, lockchart,
  VdW, TT) are largely ABSENT from 2026, so the fitted upside
  evaporates while the general-search deficit (~12 net on 2025 shared
  ground) persists and compounds. The guards mostly DECLINE cleanly
  out of distribution (no misfire disasters found) — the problem is
  not that the bands hurt, it is that the underlying general-purpose
  search+inprocessing is behind kissat's, which the 2025 suite masked.
  This re-ranks the roadmap: general-search/inprocessing quality (the
  continuous-inprocessing re-tune, the fsweep discriminator, the
  restart/trail-reuse ecology) is worth more than any further
  2025-family band.**
- **FRESH HEAD-TO-HEAD RE-BASELINE (2026-08-28/29, sequential quiet
  runs, 3600 s/16 GB/32 NUMA-balanced cores): solver12 296/400 PAR-2
  922,514 v kissat 4.0.4 292/400 PAR-2 939,548 — WE LEAD +4 SOLVED AND
  ON PAR-2** (`log/abtest-solver12-2026-08-28-08-59-09/solver12/
  results.tsv` v `log/kissat-full-20260828-162018/results.csv`; zero
  contradictions; 292 proofs verified ok + 4 checker-timeouts = the
  standing miter watch class). Unique sets 44 v 40, both-timeout 64.
  On the 252 both-solved cells kissat is wall-faster on 188 — kissat
  remains quicker per shared cell; we solve MORE (the engineered
  capability classes: php/counting x11, RoundRobin/MVRR x10, oddball
  tto_zp x6, xor/tseitin, VdW, lockchart-g2, TT496, frb80, HCP-529...).
  The 2026-08-10 reference read 292 v 294 (−2): the S26-S28c
  promotions flipped the head-to-head to +4.
- HEAD: SESSION 29 (faithful sweep banked default-off). Defaults
  unchanged since S28c. Armed-gate deal 2026-08-27: base 297. Lineage: 261 → 271 → 277
  → 280 → 286 → 290 → 292 → 293 → 294 → ~295 (S26) → 296 (S27b) →
  297/298 (S28+ deals).
- **SAT_SWEEP_FAITHFUL (S29, default OFF, modes on|armed|yield):** the
  full kissat sweep.c engine in `src/sweep_kissat.rs`. Standalone-only
  capabilities: uniqinv40prop (UNSAT 136-754 s, verified, any mode) and
  b18 (UNSAT ~3,100-3,500 s, global mode only). Casualty map and dead
  discriminator axes in the S29 section — do not re-gate without a new
  axis. Debug tooling riding along (env-gated, inert):
  SAT_DEBUG_MODEL_FILE / SAT_DEBUG_PROOF_CLAUSE / SAT_DEBUG_FSWEEP_VARS
  / SAT_FSWEEP_INVARIANTS / SAT_SWEEP_FAITHFUL_NOSUBST /
  SAT_SWEEP_FAITHFUL_EFFORT.
- kissat 4.0.4 reference: **294/400 same-host 2026-08-10**
  (`log/kissat-full-20260810-073149/results.csv`). Remaining kissat-only
  families after S28: 16x16 miters (4 — m29 now OURS via strict-auto),
  oddball residue (~4), TT (2), lockchart (2), grs (2), pj (2), b18/b19
  BMC (2), singletons (rook-51, par32-2, cfi-rigid, oisc, ER_400,
  uniqinv40, x-epic/nla-step class, ...).
- Default surface SESSIONS 15-28: SAT_VIVIFY_DEDUCE=on + _ARMED_MIN=500k;
  SAT_REPHASE_UNARMED_MIN=500_000; SAT_WALK_EFFORT_UNARMED=50;
  SAT_WALK_WARMUP_UNARMED=on; SAT_WALK_STALL_GIVEUP=16; SAT_SWEEPCOUNT=on;
  SAT_SWEEP_YIELD_ESCALATE=20 + SAT_SWEEP_YIELD_MIN_EQUIVS=1000;
  SAT_RESTART_DIVE=on (S21); SAT_RESTART_DIVE2=on (S22);
  SAT_DIVE_REUSE_TRAIL=on (S25); SAT_ELIM_BOUND_DIVE2=on (S26);
  **SAT_CHRONO_STRICT=auto (S28: dive2-band ∨ congruence-productive
  latch; on = global strict = measured bench loser; off = legacy
  guarded chrono only)**; SAT_BACKBONE=off; banded sort/tier3/reuse
  knobs off (closed).
- The deep-unarmed walk class is a managed LOTTERY SURFACE (unchanged);
  the global-restart-parity A/B is the freshest, sharpest measurement of
  that surface: +8/−16 on identical mechanisms. Judge walk members as
  class rebalance, not individual capability.
- **Same-defaults deal variance at 3600 s full bench is ±2-4 solved**;
  the paired A/B inside ONE deal is the real signal. SESSION 21's gate
  is maximally clean by construction: 396/400 cells conflict-identical,
  the delta is exactly the in-band cells.
- **Medium-1800 s baseline: still NEEDS RE-MEASUREMENT (ranked item 5);
  last measured 74/100 at c469b03.**
- Suites: `benchmarks/miterded-2026-08-02` (23 cells, the standard screen
  for late-armed-band candidates — used to pick rpmf in SESSION 21),
  `benchmarks/frontier-2026-07-30` (38), miterarmed-2026-08-01 (18).
- In-band dive cells for future deals (upside, no action needed):
  oddball_24_4 (kissat 789 s), oddball_26_4 (needs walk-min too),
  baseballcover12 (kissat-unsolved; a first-ever candidate).

## Standing traps (updated 2026-08-28 + carried)

- **SESSION 29: THE INLINE-TAG CONTRACT.** Any pass that edits clause
  content mid-search MUST either run only where inline binary tags
  never activated (armed-at-preprocess cells) or call
  `deactivate_watch_inline_tags()` first. A stale TAGGED watcher is
  trusted blindly → false propagation → (via chrono assignment-level
  max) a PERMANENT poisoned level-0 value → false UNSAT. The failure is
  silent in release (debug_asserts catch it only in debug builds) and
  surfaces hundreds of seconds later in unrelated passes.
- **SESSION 29: false-verdict debugging order.** drat-trim backward
  says only NOT VERIFIED; forward mode stops at the first
  UNVERIFIABLE line, which can be TRUE (vivify through stale learned
  clauses over ELS-eliminated vars is legitimately unverifiable-but-
  sound). To find the first FALSE derivation, use the reference-model
  audit (SAT_DEBUG_MODEL_FILE) — see the S29 method note. Deletions
  "that do not occur" in drat-trim warnings are pre-existing ELS noise
  (value-aware virtual binaries), not the bug.
- **SESSION 29: 180 s implication-oracle timeouts are useless on cells
  whose SAT side takes 2,000+ s** (the dislog bisection dead-end) —
  get one model once, then model-check candidate clauses at O(1).

- **SESSION 18:** WALL-LIMIT-ONLY-IN-CDCL bites hard — SAT_ELIM_PRODUCTIVE_
  MIN_PCT arming on RoundRobin ran 14 h with no wall stop (stuck in a
  non-CDCL elimination path). Any new mid-search-elimination trigger MUST
  carry a tick/resolvent bound or it can hang the whole bench. When probing
  at a wall limit, sanity-check `ps -o etimes` — a probe past its wall is
  wedged, kill it (bracket-trick pkill: `pkill -9 -f '[s]s pattern'`).
  Biased screens: a subset built from lottery cells will favor the config
  that helps THAT subset (1M latch 14/19) and mislead vs the full bench
  (1M LOST 286 v 291) — screen subsets must include the config's KNOWN
  casualties, and only the full 400-cell A/B decides.
- **SESSION 16b:** REACHABILITY-AUDIT LAW — before tuning any knob, trace
  its enable chain to the class it targets; three separate features this
  week (trail reuse, walk-effort-unarmed, unarmed rephase) were dead code
  on their target class because an upstream gate (arming path,
  rephase_enabled) never fired there. A `*_steps=0` or `rephases=0` stat
  on a cell the feature should touch is the tell. New walk-reroll flipper
  cells at 3600 s: VdW-23, reconf10_22, frb80-14-1 (join bp4_TCO/rbsat/
  case6/170223547* in the coin list; *170223547 now deterministically
  walk-solves at the latch — protect it).
- **SESSION 16:** when a knob screens conflict-IDENTICAL to base across a
  whole suite, suspect WIRING before verdict — trail reuse was only wired
  into the congruence arming path while its target family arms via the
  vivify-yield path. Check WHICH arming path a family takes
  (congruence_merges in the stats JSON) before scoping anything to
  "armed". Screen wins on UNSAT-grind suites do NOT transfer to the full
  bench when the mechanism also touches late-armed SAT cells — put
  known SAT casualties in the screen suite (reusefocused-2026-08-06 is
  the template). Stale doc comments lie about defaults (WATCH_POOL and
  WATCH_INLINE_BIN say "default off", both are ON) — trust env reads in
  Solver::new only.
- **SESSION 15:** the ranked-plan backbone item was STALE — commit 038f9c1
  (2026-07-15) had already killed it with kissat -s profiles; CHECK COMMIT
  MESSAGES of groundwork commits before re-ranking an old idea. Coin list
  additions: sum_of_3_cubes_37_bits_87 (SAT; stable 894,247-conflict
  trajectory when deduce-untouched, rerolls under any late-armed-band
  feature), MVRR_n14_d10_v2 (82-720 s margins at 3600 s, deep grinder at the
  wall). valves-gates is now ALSO a checker-timeout cell (verify caveat).
  4-arm screens at 3600 s on 23 cells run ~10.5 h wall, not ~3 h — plan
  accordingly; 400x2 full A/B ran ~15 h with verification.
- **SESSION 14b (carried):** NEVER `cargo build` the solver dir while ANY
  feature_ablation run is live — build to a scratch CARGO_TARGET_DIR or copy
  the binary out first. `pkill -f` with a self-matching pattern kills your
  own shell — use the `[b]racket` trick. ELS threshold gates ONLY the root
  standalone pass. `SAT_WALK` env name is PARKED (denylist).
- **SESSION 14b (carried):** reduce-law deep-cell coin exposure at 3600 s:
  rbsat/case6/170223547-class. Judge as coins, not capability.
- **SESSION 14 (carried):** full-bench 3600 s and medium-1800 s are separate
  ledgers. `ulimit -v` kills on VIRTUAL memory. rc-6 = allocator abort.
  SAT_LIMIT_WALL_SEC honored only in the CDCL loop.
- **Carried (SESSIONS 4-13):** deal noise ±2 medium; conflicts deterministic
  across load, wall is not; marginal-cell TIMEOUT untrustworthy under 32-way
  contention (solves ARE trustworthy); flipper list rbsat / vex / oski15 /
  VdW-22 (+case6, 170223547, sum_of_3_cubes, MVRR-n14 at 3600 s); activity
  proxies mislead; FEATURES.md/CONFIG_SCHEMA.csv are STALE (read
  src/config.rs + main.rs env reads); results.tsv written at run END; stats
  JSON on stderr, timed-out runs emit none (SAT_LIMIT_CONFLICTS probes);
  heredoc scratch writes flake — use the Write tool; perf blocked (gdb
  SIGINT sampler); `rm -rf` guarded — timestamped scratch dirs.
- **Carried ER/proof laws:** RAT-scan law (verify cost = #definitions x
  maxVar); residue/retry law (never stream an aborted ER attempt);
  deletions are load-bearing; tseitin caps legacy; SAT_TSEITIN_SNAKE off.
- **Carried closed lines (do not reopen without new mechanism):**
  starved-cell tick-cadence pipeline; unscoped root ELS/PROBE/SWEEP_ROOT
  defaults; SAT_ELIM_DEF; vivify tier-split AS A STANDALONE (SESSION 15
  exception: may re-screen as deduce+tier3 inside the late-armed band,
  ranked item 1b); gbve-adopter rounds; units-only transitive; per-mille
  RANKING thresholds (percent-mass decline-is-identity gates are the
  exception); ramsey ER emission; st_659; SAT_BACKBONE default-on (zero
  yield everywhere measured — miters, and 07-15 Bubble/fixedband profiles);
  **SESSION 16 additions:** banded vivify-sort; banded tier3; armed restart
  cadence bundle (floor=1/margin=1.10); trail reuse default-on in ANY mode
  (both-modes AND focused measured — deterministic per-cell sign flips, no
  runtime discriminator); SWEEP_SUBST for uniqinv/miters (0/6 at 3600 s
  idle, 14c — threshold variants pointless when the mechanism never fires).

## solver12's capability edge (protect in rerolls)

New SESSION 29: **uniqinv40prop and b18 are OURS-capable via
SAT_SWEEP_FAITHFUL (standalone-only until the discriminator exists)** —
the first two members of the kissat-only core we can solve at all.
New SESSION 28: **m29 (booth_dadda bit29) is now a BANKED capability,
no longer a coin** — strict-auto solves it in 3 independent deals
(3,443 / 3,028 / 3,166 s, 434 s margin in the auto gate) with 1.03M
fewer conflicts than base's trajectory. Also margin-banked: bwo −193 s,
oski15 family (12 cells, up to −1,072 s), VexRiscv, ibm-2004.
**valves-gates is a two-way in-scope coin under strict-auto** (28k
merges; base margin 24 s) — do not read a valves flip as capability
movement in either direction.
Carried SESSION 26: **dmu28 = 16_16_default_mapped_ultra_and_and_dadda_mapped_bit28**
(UNSAT 2,528 s in-gate, 1,072 s margin, FIRST-EVER), **bwo_bit29** and
**sqrt-mitern169** (in-gate miter converts; sqrt169 at-wall). Carried: **Circuit_multiplier24** (SAT 1917 s;
kissat-only before), **BubbleVsPancakeSort_7_6** (UNSAT 2274 s, fat
margin; does NOT latch band 2 — protected from dive2-scoped features by
construction). Carried first-evers:
MVRoundRobin_n14_d10_v2 (NOW A COIN — protect but expect flips),
RoundRobin_n18_d15, at-least-two-vmpc_28, rphp5_050/085, clqcl_40/50_6_5 + 5
cliquecoloring siblings (SAT_PHP_REFUTE, reroll-immune), xor_op x2
(SAT_GAUSS), tseitin_n188_d3, RoundRobin_n15-n17 + MVRR x3 (gate-BVE),
oddball-tto_zp x4 + TT_C496 + TT_C406 (endgame/arming; protected by the
500k bands), Kakuro-132, HCP-529, frb80-14-1, valves-gates (checker-timeout
caveat), oddball_13_5_ttf, battleship, bivium, gto_p60, contest04,
reconf10_22, blockpuzzle, VdW-23, sted2var, bp4_BC012_IXA + bp4_TCO_IXA
(deal-marginal), boothbit29 + sqrt-mitern169 + lec_mult_CvW + shuffling-1
(14d, now deduce-accelerated 10-16%).

## Where the evidence lives

- SESSION 28: `log/abtest-auto-vs-base-2026-08-25-05-19-21` (the
  promotion gate), `log/abtest-strict-vs-base-2026-08-24-14-46-54`
  (the unscoped negative + per-cell casualty map),
  `log/abtest-strict100-vs-strict300-vs-strict1000-vs-base-2026-08-24-11-34-52`
  (4-arm chronolevels screen on `benchmarks/chronoscreen-2026-08-24`),
  `log/fullbench-chronostrict-ab-*.log` / `log/fullbench-chronoauto-ab-*.log`
  (launch logs). Scope-signal probes and x-epic diagnostics were scratch;
  key numbers recorded in the SESSION 28 section and the solver README.
- SESSION 15: `log/abtest-cand-vs-base-2026-08-03-10-13-35` (THE verdict),
  `log/abtest-ded-vs-bbded-vs-bb-vs-base-2026-08-02-17-45-21` (miterded
  screen), `log/miterded-screen-20260802-174521.log`,
  `log/fullbench-ded-ab-20260803-101334.log`; tier-1 probes in scratch were
  transient — key numbers recorded above and in the solver README entry.
- SESSION 14d/14c/14b: `log/abtest-cand-vs-base-2026-08-01-20-32-12`,
  `log/seedgate-s14c-confirm-2026-08-01-00-07-44`,
  `log/abtest-cand-vs-base-2026-07-31-06-41-31`.
- Mechanism deep dives: `plan/kissat-gaps.md` (NOTE: its backbone/probing
  "small ports" ranking is now measured-refuted for the miter class),
  `plan/gap-read-full-2026-07-30.md`, `plan/gap-read-2026-07-21.md`.
- SESSIONS 4-13 full text: git history of this file (up to 52a8f95);
  14b/c/d full text up to 93ab682.
