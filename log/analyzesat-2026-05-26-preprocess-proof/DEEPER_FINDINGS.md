# Deeper findings: clause order, BSR safety, and proof overhead

## What changed in this investigation

Earlier AnalyzeSAT passes looked at lucky units, restarts/EMA, clause minimization, binary-fast, OTFS, and branch/phase policy. This pass tested a different boundary: how the solver constructs and logs the initial/original clause database before search.

The key surprise is that initial literal order is not just a parser detail. On this solver it changes watched literals, proof clauses, garbage shape, and conflict trajectory enough to swing Kakuro by about 5x in wall time while keeping preprocessing counters effectively identical.

## Matrix design

All runs used the same current binary and the same 10-instance profiling suite.

Common environment:

```text
SAT_STATS_JSON=on
SAT_TRACE_PREPROCESS=1
SAT_LIMIT_WALL_SEC=295
timeout=300s
memory=16384 MB
```

Configs:

- `default`
- `SAT_BVE=off`
- `SAT_FULL_BSR=off`
- `SAT_SIMPLIFICATION=off`
- `SAT_INITIAL_CLAUSE_MODE=input-order`
- `SAT_INITIAL_CLAUSE_MODE=raw`
- `SAT_PROOF=off`

The runner stopped a candidate config as soon as it produced `UNKNOWN` or a different result on a row that default solved. That is why the disabled-preprocessing configs have fewer rows.

## Clause-order decomposition

Kakuro is the clearest row:

| mode | result | wall | preprocess | search | conflicts | decisions | propagations |
|---|---|---:|---:|---:|---:|---:|---:|
| default | SAT | 255.761s | 34.780s | 213.797s | 732107 | 3188069 | 617655456 |
| input-order | SAT | 50.008s | 35.086s | 8.278s | 37074 | 329361 | 34528910 |
| raw | SAT | 50.714s | 35.021s | 9.350s | 37074 | 329361 | 34528910 |

The conflict-aligned traces confirm the preprocessing work was the same before search:

```text
default:     eliminated=56214 resolvents=210762 subsumed=4868640 original_clauses=14742137 original_literals=52891600
input-order: eliminated=56214 resolvents=210762 subsumed=4868640 original_clauses=14742137 original_literals=52891600
```

The divergence is in search:

```text
default final:     conflicts=732107 decisions=3188069 propagations=617655456 restarts=1533
input-order final: conflicts=37074  decisions=329361  propagations=34528910  restarts=125
```

This is not a local primitive speedup. It is a trajectory effect caused by literal/watch order. It should be handled like other chaos-sensitive SAT heuristics: gate it by formula family and validate on full-suite status, not by a single Kakuro win.

Velev is the counterexample:

| mode | result | wall | conflicts | propagation throughput ratio |
|---|---|---:|---:|---:|
| default | SAT | 77.545s | 179968 | baseline |
| input-order | SAT | 109.516s | 367810 | roughly same props/s |
| raw | SAT | 122.182s | 367810 | slower props/s |

Input/raw roughly double the conflict work on Velev, so a global default change would move risk from Kakuro to another family.

## BVE and simplification are still mandatory

`SAT_BVE=off` and `SAT_SIMPLIFICATION=off` both failed immediately on Sudoku:

```text
default: UNSAT 236.575s
SAT_BVE=off: UNKNOWN 295.77s
SAT_SIMPLIFICATION=off: UNKNOWN 295.77s
```

That is a baseline-solved status regression under the repo rule. There is no promotable path here without replacing the missing simplification power elsewhere.

## Full-BSR-off has a tempting but unsafe shape

`SAT_FULL_BSR=off`:

- Sudoku: 236.575s -> 181.211s, same UNSAT status
- Iter30: 17.477s -> 13.205s, same SAT status
- K4: 60.990s UNSAT -> 296.78s UNKNOWN

K4 work x speed:

```text
conflicts: 1607608 -> 8040136, work_ratio=5.001
props/s:   587144  -> 482412,  speed_ratio=1.217
net predicted slowdown ~= 6.087x
```

The BSR counter itself is zero on K4 in both modes, but disabling full BSR changes the final clause/literal shape enough to produce more work and more proof bytes. This argues for selected/conditional BSR scheduling, not for global off.

## Proof logging overhead is measurable and not a search effect

`SAT_PROOF=off` produced identical conflicts, decisions, and propagations to default on every row. Examples:

```text
Sudoku: conflicts=259775, decisions=824599, propagations=106152421 in both modes
K4:     conflicts=1607608, decisions=4216679, propagations=30259947 in both modes
Kakuro: conflicts=732107, decisions=3188069, propagations=617655456 in both modes
```

Wall/PAR-2 still improved:

```text
default PAR-2:   869.765
proof-off PAR-2: 729.089
delta:          -140.676s
```

The solver reports `proof_sec` as only the finalization/discard time, so the real cost is likely per-clause proof logging and formatting in the hot paths. `ProofLog::write_clause_line` sorts the proof clause before writing (`src/main.rs:951-958`), and proof logging is called from original-clause insertion, simplification, strengthening/deletion, and learned clause emission. The next proof task should instrument proof record/write time around those calls, not just final `proof_sec`.

This cannot be promoted as `SAT_PROOF=off`: UNSAT rows need `proof.out` for the SAT Competition interface. It is a performance target for proof logging implementation.

## Perf limitation

`perf stat` was attempted with cycles/instructions/branch/cache/TLB counters and failed before running the solver:

```text
perf_event_paranoid setting is 4
```

The failure is captured in `perf-stat-attempt.stderr`. The work x speed decomposition therefore uses solver counters and search seconds from JSON stats instead of hardware counters.

## New task shape

1. New issue: gate `SAT_INITIAL_CLAUSE_MODE=raw`/`input-order` by formula-family evidence. Acceptance should require at least the profiling suite plus a medium solver10/solver11 promotion gate if the default is changed.

2. Existing BSR issue should receive this evidence: global `SAT_FULL_BSR=off` has real wins but produces a K4 UNKNOWN, so the task should focus on selected BSR scheduling or family-specific avoidance.

3. Existing proof issue should receive this evidence: proof logging overhead is a pure execution penalty with identical work counters; the next step is timing proof-record calls and replacing per-clause sort/ASCII formatting where possible while preserving DRAT output.
