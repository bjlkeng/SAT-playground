<!-- ARCHIVED 2026-05-29 — search-feature efficacy verdicts under fresh re-evaluation (bead SAT-playground-gbc). DO NOT CONSULT unless explicitly asked. -->

## Validation

Focused/stable default-profile promotion was reverted on 2026-05-24 after a clean solver 10
comparison:

```bash
SAT_STATS_JSON=on bash tools/bench.sh -t 300 -m 16384 -d benchmarks/profiling \
  --log-dir log/phase1/5b2.2.34-after-default solver/11-kissat-port
python3 tools/compare_bench.py \
  --before log/phase1/5b2.2.34-before-rerun/results.csv \
  --after log/phase1/5b2.2.34-after-default/results.csv \
  --timeout 300
```

| Run | Settings | Solved | SAT | UNSAT | Timeouts | PAR-2 | Results |
|---|---|---:|---:|---:|---:|---:|---|
| before focused restart logn | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on SAT_STATS_JSON=on` | 9/10 | 5 | 4 | 1 | `1756.406` | `log/phase1/5b2.2.34-before-rerun/results.csv` |
| default after promotion | `SAT_STATS_JSON=on` | 9/10 | 5 | 4 | 1 | `1220.197` | `log/phase1/5b2.2.34-after-default/results.csv` |

The promoted run improves aggregate PAR-2 by `536.209` under the Phase 1 PAR-2-only promotion
rule. Instance churn is recorded for diagnosis: `case9` changed from timeout to SAT in `1.321s`,
while `mp1` changed from SAT in `255.953s` to timeout. The largest wins were `case9`
(`-298.679s`), K4 (`-109.586s`), and battleship (`-101.356s`).

Follow-up clean 300s / 16 GiB comparison against solver 10 showed the promoted focused/stable
default was not acceptable as a global default:

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| solver 10 default | `solver/10-bve-preprocess` | 10/10 | `699.671` | `log/phase1/solver10-default-300-vs-solver11-clean/results.csv` |
| solver 11 focused/stable default | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 9/10 | `1201.538` | `log/phase1/5b2.2.52-s11-default-clean/results.csv` |
| solver 11 single/no-LBD | `SAT_USE_LBD=off SAT_SEARCH_MODE=single SAT_MODE_USE_TICKS=off SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `798.196` | `log/phase1/5b2.2.52-s11-single-nolbd-clean/results.csv` |

The default and fast profiles therefore use single-mode/no-LBD search until the focused/stable stack
or a replacement default clears the solver 10 baseline.

Restart trail reuse remains default-off after the 2026-05-25 per-mode split. The implementation now
keeps the Kissat stable reuse rule inside focused/stable mode instead of applying it to the
solver-10-compatible single-mode Luby path. That fixes the previous `mp1` `UNKNOWN`: the exact
`SAT_RESTART_REUSE_TRAIL_STABLE=on` single-mode rerun now follows the normal single-mode restart
path and solves `mp1`, while focused/stable runs can still enable stable or focused reuse explicitly.

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| default after per-mode reuse controls | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `748.732` | `log/phase1/5b2.2.55-default-after/results.csv` |
| stable-only reuse, stopped on hard failure | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_RESTART_REUSE_TRAIL_STABLE=on` | 3/4 before stop | `FAIL` | `log/phase1/5b2.2.55-reuse-stable-after/results.csv` |
| stable-only reuse after focused/stable scoping | `SAT_RESTART_REUSE_TRAIL_STABLE=on SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` on mp1 | 1/1 | `42.419` | `log/bench-11-kissat-port-2026-05-25-18-43-57/results.csv` |

Focused/stable phase-map controls landed on 2026-05-25 as experiment knobs. The first matrix was
rejected because the env-facing focused/stable path used a VSIDS-in-focused hybrid and the VMTF
batch bump order did not match Kissat. After the focused/stable default was changed to focused-only
VMTF and analyzed variables are moved in existing queue-stamp order, the prior current-auto Velev
`UNKNOWN` rerun solves with the actual focused/stable/tick path enabled.

| Run | Settings | Solved before stop | Outcome | Results |
|---|---|---:|---|---|
| default after phase-map controls | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | PAR-2 `755.634`; no status regression vs `5b2.2.55-default-after` | `log/phase1/5b2.2.54-default-after/results.csv` |
| current auto/auto mapping | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` | 6/7 | rejected: velev SAT to `UNKNOWN` | `log/phase1/5b2.2.54-current-auto-after/results.csv` |
| saved/saved mapping | `SAT_FOCUSED_PHASE=saved SAT_STABLE_PHASE=saved` plus focused/stable/ticks | 3/4 | rejected: mp1 SAT to `UNKNOWN` | `log/phase1/5b2.2.54-saved-saved-after/results.csv` |
| saved/best mapping | `SAT_FOCUSED_PHASE=saved SAT_STABLE_PHASE=best-then-target-then-saved` plus focused/stable/ticks | 6/7 | rejected: velev SAT to `UNKNOWN` | `log/phase1/5b2.2.54-saved-best-after/results.csv` |
| target/best mapping | `SAT_FOCUSED_PHASE=target-then-saved SAT_STABLE_PHASE=best-then-target-then-saved` plus focused/stable/ticks | 1/2 | rejected: Sudoku UNSAT to `UNKNOWN` | `log/phase1/5b2.2.54-target-best-after/results.csv` |
| current auto/auto after focused VMTF default | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` on velev | 1/1 | SAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-18-29-53/results.csv` |

A rejected UNKNOWN cleanup attempt on 2026-05-25 normalized rejected knobs to the safe single-mode
path. That quarantine was reverted. The current fix keeps the env-facing code paths enabled and
addresses the underlying issues: focused/stable defaults to focused-only VMTF, focused VMTF
batches preserve Kissat queue-stamp order, conflict-triggered mode switches happen at the
post-propagation scheduling boundary, stable restart reuse no longer applies to the single-mode
Luby path, focused mode includes Kissat random decision sequences and phase cycling, and stable
reluctant restarts use Kissat's `1024` conflict scale.

| Rerun | Settings | Scope | Outcome | Results |
|---|---|---|---|---|
| focused/stable VMTF actual-path fix | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on SAT_VMTF=focused-only` | Sudoku | UNSAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-18-19-49/results.csv` |
| focused/stable auto actual-path fix | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` | velev | SAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-18-29-53/results.csv` |
| restart reuse actual-path fix | `SAT_RESTART_REUSE_TRAIL_STABLE=on` | mp1 | SAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-18-43-57/results.csv` |
| focused/stable case9 actual-path fix | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on` | case9 | SAT, `0` unknown | `log/bench-11-kissat-port-2026-05-25-19-57-35/results.csv` |
| full focused/stable actual-path profile | `SAT_USE_LBD=on SAT_SEARCH_MODE=focused-stable SAT_MODE_USE_TICKS=on SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10 profiling instances | 10/10, PAR-2 `938.956`, `0` unknown | `log/bench-11-kissat-port-2026-05-25-20-01-30/results.csv` |
| final default profile check | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10 profiling instances | 10/10, PAR-2 `757.854`, `0` unknown | `log/bench-11-kissat-port-2026-05-25-20-22-29/results.csv` |

Formula classification landed as instrumentation on 2026-05-25. The default path remains
status-safe; adaptive feature routing is still future work because binary-fast and VMTF are
construction-time choices in the current solver.

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| before classifier | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `755.634` | `log/phase1/5b2.2.54-default-after/results.csv` |
| after classifier | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `753.747` | `log/phase1/f06-classify-default-after/results.csv` |

Lucky assignment landed on 2026-05-25 as a pre-search SAT fast path, then was demoted back to
opt-in after a rerun showed the default-on pass solved only the battleship row and added more time
across the rest of the profiling suite. The canonical battleship case remains available with
`SAT_LUCKY=on`, but default and fast profiles leave it off until an adaptive gate proves out.

| Run | Settings | Solved | PAR-2 / Time | Results |
|---|---|---:|---:|---|
| before lucky default | default | 10/10 | PAR-2 `859.447` | `log/bench-11-kissat-port-2026-05-25-21-18-03/results.csv` |
| after lucky default | default | 10/10 | PAR-2 `831.021` | `log/bench-11-kissat-port-2026-05-25-22-30-00/results.csv` |
| rerun lucky off | `SAT_LUCKY=off` | 10/10 | PAR-2 `759.720` | `log/analyzesat-2026-05-25-rerun/FINDINGS.md` |
| after lucky demotion | default | 10/10 | PAR-2 `749.356` | `log/phase1/3fs-lucky-off-default-profile/results.csv` |
| battleship acceptance | `SAT_LUCKY=on`, 60s one-instance run | 1/1 | `0.089s`, `lucky_solved=1` | `log/bench-11-kissat-port-2026-05-25-22-29-54/results.csv` |
| solver 10 gate | solver10 clean vs previous solver11 vs candidate | 10/10 candidate | `promotion_gate=FAIL`; candidate still `+131.350` PAR-2 vs solver10 | `tools/check_solver11_promotion.py --solver10 log/phase1/solver10-default-300-vs-solver11-clean/results.csv --previous log/bench-11-kissat-port-2026-05-25-21-18-03/results.csv --candidate log/bench-11-kissat-port-2026-05-25-22-30-00/results.csv --timeout 300 --memory-mb 16384` |
| demotion gate | solver10 clean vs previous solver11 vs lucky-off candidate | 10/10 candidate | `promotion_gate=FAIL`; candidate improved previous solver 11 by `63.588` PAR-2 but still lost solver10 by `49.113` PAR-2 | `tools/check_solver11_promotion.py --solver10 log/phase1/3fs-solver10-default-profile/results.csv --previous log/bench-11-kissat-port-2026-05-25-23-45-39/results.csv --candidate log/phase1/3fs-lucky-off-default-profile/results.csv --timeout 300 --memory-mb 16384` |

Any future default/fast promotion must pass the solver 10 gate on the same benchmark set:

```bash
python3 tools/check_solver11_promotion.py \
  --solver10 log/phase1/solver10-default-300-vs-solver11-clean/results.csv \
  --previous log/phase1/5b2.2.53-after-default-rollback/results.csv \
  --candidate log/phase1/<candidate>/results.csv \
  --timeout 300 \
  --memory-mb 16384
```

Solver 11 single-mode parity follow-up on 2026-05-24 kept two low-risk runtime fixes:

- SAT model files are still emitted for SAT, but the internal full-CNF reparse/check now runs only
  with `SAT_CHECK_INVARIANTS=on`; normal benchmark runs record `model_check_result=not_checked`
  and rely on the existing harness assignment verification.
- Propagation is specialized on `SAT_BINARY_FAST`, so the default binary-fast-off path compiles out
  the per-propagation binary implication branch. Binary-fast runs preserve the configured
  clause-minimization mode unless `SAT_CLAUSE_MIN=off` is explicit.

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| solver 11 rollback baseline | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `785.980` | `log/phase1/5b2.2.53-after-default-rollback/results.csv` |
| after model-check gating | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `783.169` | `log/phase1/5b2.2.56-after-model-check-gate/results.csv` |
| after binary-fast propagation specialization | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `745.558` | `log/phase1/5b2.2.56-after-prop-specialization/results.csv` |
| rejected accounting/LBD helper hoist | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `750.454` | `log/phase1/5b2.2.56-after-prop-accounting-hoist/results.csv` |
| rejected input-hash opt-in | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `746.133` | `log/phase1/5b2.2.56-after-input-hash-optin/results.csv` |

The accepted state improves solver 11 by `40.422` PAR-2 against the rollback baseline, with no
status changes or correctness failures. It still trails the solver 10 clean baseline by `45.887`
PAR-2, mainly on sudoku (`+13.574s`), Kakuro (`+14.878s`), and `case9` (`+6.434s`).

The 2026-05-28 execution-tax follow-up specialized normal-search propagation accounting and
enqueue policy checks behind a `NORMAL_SEARCH` const generic. This compiles the ordinary search
path down to direct propagation accounting, saved-phase writes, and root-trail checks while keeping
temporary-assumption accounting on the runtime-policy path. The source-level input is
`log/analyzesat-2026-05-28-exec-tax/FINDINGS.md`.

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| before normal-search specialization | default | 10/10 | `753.236` | `log/bench-11-kissat-port-2026-05-28-20-20-07/results.csv` |
| first after normal-search specialization | default | 10/10 | `734.833` | `log/bench-11-kissat-port-2026-05-28-20-51-33/results.csv` |
| required post-rebase retest | default | 10/10 | `750.260` | `log/bench-11-kissat-port-2026-05-28-21-15-51/results.csv` |
| solver 10 comparison gate, post-rebase retest | solver10 clean vs previous solver11 vs candidate | 10/10 candidate | `promotion_gate=FAIL`; candidate improved previous solver 11 by `2.976` PAR-2 but still lost solver10 by `50.589` PAR-2 | `tools/check_solver11_promotion.py --solver10 log/phase1/solver10-default-300-vs-solver11-clean/results.csv --previous log/bench-11-kissat-port-2026-05-28-20-20-07/results.csv --candidate log/bench-11-kissat-port-2026-05-28-21-15-51/results.csv --timeout 300 --memory-mb 16384` |

Both after runs were status-safe: `compare_bench.py` reported no correctness failures and no status
changes. The first run showed PAR-2 `-18.403`, with the largest wins on Kakuro (`-9.034s`), sudoku
(`-5.855s`), velev (`-1.790s`), and `case9` (`-1.425s`). The required post-rebase retest was much
noisier, at PAR-2 `-2.976`: Kakuro (`-3.376s`), velev (`-1.269s`), and `case9` (`-1.562s`) still
improved, while sudoku regressed by `+2.017s`. Treat this as a low-risk hot-path cleanup with
status-safe evidence, not as a completed solver10 parity gate. Because solver 11 still trails solver
10, the remaining residual gap is tracked separately as a perf-counter/layout question.

`SAT_VMTF=single` was added as a default-off diagnostic experiment on 2026-05-25. An initial
unbounded version was rejected because `UNKNOWN` means the solver produced neither SAT nor UNSAT
and is therefore a benchmark failure, even if some rows improve. This single-mode route is not the
Kissat-faithful VMTF policy and is not a promotion target; the supported Kissat-like path is
focused-mode VMTF inside focused/stable search. The table below is retained as historical evidence
for the single-mode experiment and for why focused-mode parity became the next implementation
target.

| Run | Settings | Solved | PAR-2 | Results |
|---|---|---:|---:|---|
| before VMTF-single work | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `745.558` | `log/phase1/5b2.2.56-after-prop-specialization/results.csv` |
| after VMTF-single work, default | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `755.000` | `log/phase1/egy-default-no-regression/results.csv` |
| rejected unbounded VMTF single-mode branch queue | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_VMTF=single` | 5/10 | `3156.865` | `log/phase1/egy-vmtf-single-profile/results.csv` |
| current default after guarded VMTF | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295` | 10/10 | `755.239` | `log/phase1/egy-default-after-formulaguard-profile/results.csv` |
| guarded VMTF single-mode branch queue | `SAT_STATS_JSON=on SAT_LIMIT_WALL_SEC=295 SAT_VMTF=single` | 10/10 | `647.341` | `log/phase1/egy-vmtf-single-formulaguard-profile/results.csv` |

Default behavior had no status churn and the default-off `755.239` PAR-2 run was consistent with
the earlier `755.000` default-off rerun. The guarded single-mode experiment was profile-positive on
that run and had no `UNKNOWN`/timeout/error rows, but it remains a non-Kissat diagnostic path rather
than a default or fast-profile candidate.

Latest correctness checks for the accepted state:

- `cargo test`: 328 passed
- `cargo clippy --all-targets -- -D warnings`: passed
- `bash tools/smoke_test.sh solver/11-kissat-port`: 9/9 passed
- `SAT_CHECK_INVARIANTS=on bash tools/smoke_test.sh solver/11-kissat-port`: 9/9 passed
- `SAT_VMTF=single bash tools/smoke_test.sh solver/11-kissat-port`: 9/9 passed
- `bash -n tools/bench.sh`: passed
- `python3 tools/validate_solver_result.py --self-test`: passed
- `python3 tools/compare_bench.py --self-test`: passed
- `python3 tools/validate_solver11_plan.py`: passed
- comparison verdict against rollback: `significant_improvement`, `PASS`

Run on 2026-05-08:

```bash
cargo test
bash tools/smoke_test.sh solver/10-bve-preprocess
```

Results:

- `cargo test` in `solver/10-bve-preprocess`: 45 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-08-16-08-17`

Rerun after the large-formula BSR gate on 2026-05-08:

- `cargo test` in `solver/10-bve-preprocess`: 45 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-08-20-35-04`

Rerun after the MiniSat-style persistent preprocessing loop on 2026-05-08:

- `cargo test` in `solver/10-bve-preprocess`: 45 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-08-23-35-41`

Latest rerun after the lazy deleted-clause watcher cleanup on 2026-05-09:

- `cargo test` in `solver/10-bve-preprocess`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-09-15-38-19`

Latest rerun after the 2026-05-15 simplification data-layout pass:

- `cargo test` in `solver/10-bve-preprocess`: 48 passed
- smoke suite: 9/9 passed, including DRAT verification for all UNSAT smoke instances
- smoke log: `log/2026-05-15-21-45-47`

