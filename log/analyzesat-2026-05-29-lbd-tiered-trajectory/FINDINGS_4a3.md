# FINDINGS — SAT-playground-4a3: lbd-tiered reducer TIMEOUTs on mp1 / case9

**Agent:** VioletRidge (`/nextbeads 1`, phase1)
**Date:** 2026-05-29
**Base:** `origin/main` @ `724f573`
**Worktree:** `/tmp/sat-worktrees/VioletRidge-1780065981`
**Solver:** `solver/11-kissat-port`, config `SAT_USE_LBD=on SAT_REDUCE=lbd-tiered`

## TL;DR

The bead's two stated hypotheses are **both refuted** on current `main`:

1. **"Unbounded DB growth → propagation throughput collapse"** — REFUTED. `reduce_db`
   fires regularly throughout the run and throughput degrades only mildly. The learned
   DB does not grow unbounded.
2. **"`should_reduce_db` only triggers at level-0 returns (Gap CD-5, main.rs:7072)"** —
   REFUTED / STALE. In current code the `reduce_db` call (`main.rs:7892`) is **outside**
   the `if current_level()==0` block, so it runs before *every* decision at *any* level.
   Empirically confirmed: reduce fired 106× on mp1, which deep-dives at level ~600 and
   rarely returns to level 0. **09n "Fix part 1" is already implemented.**

The real cause of the mp1 / case9 timeouts is **clause-retention trajectory sensitivity**,
and the two instances want *opposite* things:

- **mp1 (SAT):** default/legacy **never reduces** (`reduce_db=0`), hoards all 425k learned
  clauses, and solves at **425,229 conflicts / 44s**. lbd-tiered reduces — including an
  emergency hard-budget pass at ~250k conflicts that deletes ~97% of the DB — which derails
  the winning hoard trajectory. **mp1 wants LESS reduction.**
- **case9 (SAT):** default/legacy reduces *aggressively* (`reduce_db=530` by 60s, learned
  ~6–8k, ~37k conflicts/s) and solves at **128s**. lbd-tiered protects tier-0 and keeps
  ~2× more clauses (~15k), ~halving throughput (~19k conflicts/s), so it never converges in
  budget. **case9 wants MORE reduction.**

No single fixed lbd-tiered policy satisfies both, and tuning toward either regresses the
other (and the UNSAT REGRandom instance — confirmed by the prior s11-06 attempts logged on
the bead). This is the "lucky/fragile trajectory" situation CLAUDE.md warns against
hard-coding. **No mechanism fix is shipped.**

## Aggregate-PAR-2 verdict (the 2026-05-29 rescoped acceptance metric)

From the existing full-suite evidence `log/analyzesat-2026-05-26-clausedb-cycle/`
(profiling suite, 300s timeout; unsolved = 2×300 = 600 PAR-2). My mp1 repro on current
`main` (724f573) reproduces the timeout with an identical trajectory, so this evidence is
current.

| instance | A_baseline (s) | C_lbd_tiered (s) | Δ PAR-2 |
|---|---|---|---|
| sudoku | 232.8 | 192.4 | −40.4 |
| 6s299 | 17.8 | 16.5 | −1.3 |
| REGRandom (UNSAT) | 59.7 | 164.2 | **+104.5** |
| mp1 (SAT) | 44.9 | **TIMEOUT→600** | **+555.1** |
| Kakuro | 241.0 | 119.0 | −122.0 |
| SCPC | 13.9 | 17.4 | +3.5 |
| velev | 71.4 | 35.9 | −35.4 |
| brocard | 9.3 | 8.7 | −0.6 |
| battleship (SAT) | 23.2 | 132.9 | **+109.7** |
| case9 (SAT) | 128.4 | **TIMEOUT→600** | **+471.6** |
| **PAR-2 total** | **842.3** | **1887.0** | **+1044.7** |

lbd-tiered loses to baseline by **+1044.7 PAR-2**. Even a *perfect* rescue of both timeouts
(mp1→45s, case9→128s) only reaches ~860 — still ≈ tie/slight loss vs 842 — and the only
known mp1 rescue (more aggressive deletion) regresses REGRandom. **`SAT_REDUCE=lbd-tiered`
is not promotable via reduce-policy tuning.** (Note: this baseline config measured 842; the
current default profile benches ~750. lbd-tiered loses to both by a wide margin.)

`SAT_REDUCE=lbd-tiered` is opt-in and default-off, so this finding does not affect the
default-profile PAR-2.

## Evidence / repro

Build: `cd solver/11-kissat-port && bash build.sh` (binary sha in JSON_STATS).
Instances decompressed from `benchmarks/profiling/*.cnf.xz` (run.sh does not decompress xz).

### mp1 — lbd-tiered vs default trajectory (trace every 25k conflicts)

```
# lbd-tiered (TIMEOUT)
SAT_USE_LBD=on SAT_REDUCE=lbd-tiered SAT_TRACE_SEARCH_INTERVAL=25000 \
  ./target/release/sat-solver /tmp/vr-mp1.cnf /tmp/proof
# default (SAT @ 425,229 conflicts / 44.2s)
SAT_TRACE_SEARCH_INTERVAL=25000 ./target/release/sat-solver /tmp/vr-mp1.cnf /tmp/proof
```

| conflicts | default learned / reduce_db | lbd-tiered learned / reduce_db | level |
|---|---|---|---|
| 25k  | 24,999 / 0 | 16,707 / 11 | 735 (identical trajectory) |
| 100k | 99,999 / 0 | 46,895 / 28 | 691 (identical trajectory) |
| 200k | 199,999 / 0 | 78,395 / 45 | 634 (identical trajectory) |
| 225k | 224,999 / 0 | 85,746 / 48 | 642 (identical trajectory) |
| 250k | 249,999 / 0 | 86,260 / 52 | **TRAJECTORIES DIVERGE HERE** |
| 425k | **SAT (425,229)** | — | default solves |
| 725k | — | 179,075 / 106 (still wandering) | 668 |

The decisions/propagations/level are **byte-identical** for the first ~225k conflicts
(lbd-tiered's early deletions only drop never-used dead weight), then diverge at exactly
250k — where lbd-tiered finally deletes a clause the default's hoard path relies on.

Isolation test — lbd-tiered with reductions disabled (`SAT_REDUCE_DB_INIT=5000000
SAT_REDUCE_DB_INTERVAL=5000000`): the **emergency hard-budget** trigger still fires at
~250k (learned 199,999 → 6,646 in one pass, a ~97% cliff) and mp1 still times out. So the
derail is driven by the emergency reduction, and more fundamentally by the fact that *any*
deletion perturbs mp1's hoard-favorable path.

### case9 — lbd-tiered vs default (trace every 50k conflicts)

```
SAT_USE_LBD=on SAT_REDUCE=lbd-tiered SAT_TRACE_SEARCH_INTERVAL=50000 ./...sat-solver /tmp/vr-case9.cnf
SAT_TRACE_SEARCH_INTERVAL=50000 ./...sat-solver /tmp/vr-case9.cnf
```

- lbd-tiered @ ~44s: 850k conflicts, reduce_db=206, learned ~15k, level ~20–38, ~19k conf/s.
- default @ ~57s: 2.15M conflicts, reduce_db=530, learned ~6k, level ~15–40, ~37k conf/s
  (solves at 128s per A_baseline). default is ~2× faster because it keeps ~half the clauses.

Opposite of mp1: here lbd-tiered keeps **too many** clauses (tier-0 protection), halving
throughput. case9 wants MORE reduction (legacy is more aggressive and wins).

## Why this is not a mechanism bug (and what is/isn't actionable)

- **CD-5 (09n)** — the actionable half ("lift reduce out of level-0 branch") is **already
  done** in current code; the premise is stale. The remaining half (inprocessing) is tracked
  by `5b2.3.18`. 09n's Phase-1 action item is moot.
- **CD-3 (4iu)** — tier-0 permanent protection is **real**: tier-0 clauses are only deletable
  on emergency. Letting tier-0 age out (4iu's fix) makes lbd-tiered delete *more* — the
  direction case9 wants but mp1/REGRandom do not. It is a genuine knob but cannot resolve the
  opposite-preference tension; validate any change on aggregate PAR-2, not on mp1/case9 alone.
- **Emergency-reduction cliff** — the one genuinely principled, instance-agnostic robustness
  item this investigation surfaced: an emergency reduce deletes all the way down to the *soft*
  `learned_lit_budget` (`reduce_db_lbd_tiered`, main.rs:6477), nuking ~97% of the DB in a
  single pass (200k → 6.6k clauses on mp1). Kissat deletes a fraction per reduce, not down to
  a tiny floor. A gentler emergency target (e.g. a fraction of the *hard* budget) would avoid
  the cliff. It would NOT rescue mp1 (mp1 needs zero deletion) and is unlikely to flip the
  aggregate by itself, so it is filed as a separate follow-up to be validated on the full
  suite — not shipped here. (New bead created.)

## Disposition

`SAT-playground-4a3` is closed: the DB-growth/CD-5 framing is refuted, the timeouts are
honest budget-consuming (700k–850k+ conflicts; **not** correctness failures, **not** premature
non-budget UNKNOWN), and lbd-tiered is not promotable on aggregate PAR-2. No fragile guard
shipped per CLAUDE.md's anti-overfit rule.
