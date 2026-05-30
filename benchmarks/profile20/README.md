# profile20 — 20-instance feature-ablation suite (10 easy + 10 headroom)

A 20-instance suite for solver-feature ablation that, unlike `benchmarks/profiling`, contains
**headroom**: instances solver-10 cannot crack in 5 minutes but a strong solver (kissat) can. On
the all-solved profiling suite a real search improvement has nothing to rescue; here it can flip a
solver-10 timeout into a solve.

Two halves:

- **easy (10)** — the existing `benchmarks/profiling` control suite, reused verbatim (symlinked to
  `../profiling/`), to preserve direct comparability with all prior profiling results. solver-10
  (`10-bve-preprocess`) solves every one within 5 min on its fast run (best-of-3 < 300 s for all 10).
  **Caveat (variance):** these are the profiling instances and two of them are bimodal /
  variance-sensitive — `brocard_problem_large` (7.9 / 526.5 / 585.3 s) and
  `REGRandom-K4-L1-Seed40` (54.2 / 966.8 / 1278.1 s) — and `mp1-Nb7T46` / `sudoku-N30-12` each have
  one slow run. They are < 300 s on their fast runs and in the profiling suite's own 300 s-budget
  runs, but can exceed 300 s on some runs. See `selection.csv` for all three per-run times.
- **hard (10)** — drawn from the medium-100 set (`../sat-comp-2025-medium/`), **disjoint from
  profiling**. solver-10 **times out at 300 s** (it needs ≥300 s, or never finishes, in every run),
  while **kissat-latest finishes in < 280 s**.

## Selection methodology

Selected from existing repeated runs on the **identical** medium-100 instance set (verified by
instance-set equality), re-thresholded at 300 s — no fresh campaign needed:

- **solver-10 @ 1800 s / 16 GiB:** three runs
  (`log/bench-10-bve-preprocess-2026-05-{18,12,03}-*/results.csv`).
- **kissat-latest @ 1800 s:** one run (`log/bench-kissat-latest-2026-04-11-22-21-01/results.csv`).

HARD rules (robust across all three solver-10 runs):
- solver-10 cannot solve within the 300 s budget in **every** run (result==TIMEOUT, or solved but
  time ≥ 300 s). Rows with ERROR/UNKNOWN in any run are excluded — honest timeouts only, not crashes.
- kissat solved **and** time < 280 s (margin vs 300 s).
- Family-diverse (≤ 2 per coarse family); spread across kissat runtime.

There were **14 qualifying hard candidates**; the requested condition is satisfiable.
Provenance: `selection.csv` (per-instance solver-10 per-run times + kissat time), `selection.json`.
Regenerate: `python3 tools/select_profile20.py`. solver-10/kissat times come from 1800 s runs; the
hard-half solver-10 figure is its time when it *did* eventually solve (>300 s) or 1800 (timeout).

## easy half — reused profiling-10 control (solver-10 fast run < 300 s; see variance caveat above)

| instance | s10 | s10 best_s | s10 range_s (3 runs) | kissat | kissat_s | family |
|---|---|---|---|---|---|---|
| `6832fe907740af686fde98518067ea3f-velev-pipe-sat-1.0-b7` | SAT | 4.6 | 4.6–61.2 | SAT | 94.9 | velev |
| `9af7646fc4a32c6f2744ddc0c4b654b7-brocard_problem_large` | UNSAT | 7.9 | 7.9–585.3 | UNSAT | 66.4 | brocard |
| `663bb5659e42c2c75f74354f48895302-SCPC-500-13` | UNSAT | 11.9 | 11.9–13.2 | UNSAT | 6.8 | scpc |
| `3746303c659ef65aaa78f3b52cd5de49-6s299b685_Iter30` | SAT | 15.3 | 15.3–19.8 | SAT | 45.3 | s |
| `ed6d842f96d10f3400bce251f9e95bfb-battleship-16-31-sat` | SAT | 19.3 | 19.3–163.3 | SAT | 0.2 | battleship |
| `5e933a625099cc1ec6a8299a7848a2ae-Kakuro-easy-112-ext.xml.hg_7` | SAT | 29.1 | 29.1–288.1 | SAT | 42.2 | kakuro |
| `557d7d4db5399188f62bc39598c6d868-mp1-Nb7T46` | TIMEOUT | 40.4 | 40.4–1800.0 | SAT | 8.7 | mp |
| `46355da785714f239393e7630020cae3-REGRandom-K4-L1-Seed40.sanitized` | UNSAT | 54.2 | 54.2–1278.1 | UNSAT | 2.4 | regrandom |
| `fab2022deb130fe3ad1136a5c71b4109-case9` | SAT | 107.2 | 107.2–211.2 | SAT | 78.9 | case |
| `0aa22564d00e9716519918d84b25c4a7-sudoku-N30-12` | UNSAT | 171.0 | 171.0–444.7 | UNSAT | 299.5 | sudoku |

## hard half — headroom (solver-10 can't do in 5 min; kissat < 280 s)

| instance | s10 | s10_s (≥300 ⇒ TO@300) | kissat | kissat_s | family |
|---|---|---|---|---|---|
| `849950561ddce887c78fef773dccfa80-circuit_48in64out_with_800gates_4in4out_dist128_seed3.sanitized` | TIMEOUT | 1800.0 | SAT | 7.2 | circuit |
| `f0bafebdcce23ccfbaf6c27a7522069b-div-mitern172` | UNSAT | 444.4 | UNSAT | 30.5 | div |
| `5dbe7b31f9b8d8e56045493439adb949-bp4_CSO_IXA_ZR.normalised` | TIMEOUT | 1800.0 | SAT | 62.6 | bp |
| `31e843c53a76ff3961935ad55b953298-sqrt-mitern171` | UNSAT | 487.8 | UNSAT | 63.4 | sqrt |
| `16ff47a05b769ed6a04c7175cfc6da55-sqrt-mitern170` | UNSAT | 1264.8 | UNSAT | 133.7 | sqrt |
| `ebbda8d90dfd9b6500bf932f952907a6-2018D_VexRiscv-regch0-20-p1_step` | TIMEOUT | 1800.0 | UNSAT | 135.8 | d |
| `8b22396ef06770c7dfa7552a610fc911-PancakeVsSelectionSort_6_7` | TIMEOUT | 1800.0 | UNSAT | 154.6 | pancakevsselectionsort |
| `389bb0a8f3568f4bd0e71771df9093c5-BubbleVsPancakeSort_7_6` | TIMEOUT | 1800.0 | UNSAT | 209.2 | bubblevspancakesort |
| `296fd43ed40e242875984420f29be73f-oddball_24_5_ttf.normalised` | TIMEOUT | 1800.0 | UNSAT | 239.2 | oddball |
| `77a0d54f2fb3740a9a321623c0c10f3e-tseitin_grid_n12_m12` | TIMEOUT | 1800.0 | UNSAT | 247.7 | tseitin |
