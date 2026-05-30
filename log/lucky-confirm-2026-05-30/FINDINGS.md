# SAT_LUCKY promotion confirmation — 2026-05-30 (bead SAT-playground-70h)

PROMOTED: `SAT_LUCKY` on by default in the `default`/`fast` profiles (`baseline` keeps it off).

## Aggregate confirmation (n>=3 fresh, warm-up + interleaved, same binary, 300s/16GiB)
lucky-off: 735.234 / 735.700 / 731.826  (mean 734.25)
lucky-on : 721.595 / 722.391 / 722.948  (mean 722.31)   delta = -11.94
Combined with the gbc campaign n=2 (off 737.665/734.765, on 726.023/723.444): n=5 each,
lucky-on (721.6-726.0) is ENTIRELY below lucky-off (731.8-737.7) — zero overlap, ~-12,
far beyond the ~3-PAR-2 noise band. All runs 10/10, correctness clean (verified, no wrong/UNKNOWN).

## Shuffle gate (battleship, clause+literal order shuffled, 5 seeds) — PASS
lucky-on : 0.08s on every seed (order-INVARIANT; deterministic robust solve)
lucky-off: 24.4 / 416.7 / 18.3 / 903.9 / 658.6s (order-fragile; would TIMEOUT at 300s on 3/5)
=> the win is NOT an input-order coincidence; lucky removes battleship's order-fragility.

## Solver-10 gate (check_solver11_promotion.py)
candidate_minus_previous_solver11 = -8.878 (improves prior default; n=5 mean -12)
candidate_minus_solver10 = +23.277 (still trails solver-10 699.671, but prior default also did; gap narrows)
decision = candidate_improves_previous_solver11_but_loses_solver10 -> PROMOTE per aggregate-PAR-2-only policy.

## Mechanism
lucky's pre-search all-true/all-false/forward-backward probes deterministically find a model for
battleship in 0.08s. The probe overhead on the other 9 instances is small (+0.0..+2.6s each); the
net is -12 PAR-2 on this suite, and the battleship benefit is robust to input reordering.
