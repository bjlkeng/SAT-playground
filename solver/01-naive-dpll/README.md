# 01-naive-dpll

Naive DPLL solver with unit propagation, recursive branching, and backtracking. No CDCL, no watched literals, no heuristics.

## Features

- Unit propagation to fixpoint, recursive branching with backtracking
- DRAT proof output (empty clause) for UNSAT, verified by drat-trim
- 8 Rust unit tests, all 8 smoke tests pass

## Code-Level Optimization Log

All optimizations below are purely code-level (compiler flags, data representation, allocation elimination). No algorithmic changes were made.

### Environment

- **CPU:** AMD Ryzen 5 5600 6-Core (12 threads), 3.5 GHz base / 4.47 GHz boost
- **L1d/L1i:** 192 KiB (6 instances each)
- **L2:** 3 MiB (6 instances), **L3:** 32 MiB
- **RAM:** 64 GB DDR4
- **OS:** Ubuntu 22.04, Linux 6.8.0-107-generic
- **Rust:** 1.94.1 (2026-03-25)
- **Benchmark suite:** 6 profiling instances (3 crypto Feistel, 3 random 3-SAT), 120s timeout, PAR-2 scoring

### Baseline (original code, `cargo build --release` defaults)

| Instance | Type | Vars | Clauses | Result | Time |
|----------|------|------|---------|--------|------|
| feistel_b64_k32_r8 | crypto | 1120 | 3968 | SAT | 15.61s |
| feistel_b64_k32_r10 | crypto | 1376 | 4928 | TIMEOUT | >120s |
| feistel_b64_k32_r12 | crypto | 1632 | 5888 | TIMEOUT | >120s |
| random_v110_s1 | 3-SAT | 110 | 469 | UNSAT | 6.86s |
| random_v130_s3 | 3-SAT | 130 | 555 | SAT | 53.83s |
| random_v140_s1 | 3-SAT | 140 | 597 | TIMEOUT | >120s |

**Baseline PAR-2: 796.29 (3/6 solved)**

### Iteration 1: Compiler Flags

Added release profile optimizations to `Cargo.toml`:
```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
overflow-checks = false
```
Also set `RUSTFLAGS="-C target-cpu=native"` in `build.sh`.

| Instance | Result | Time |
|----------|--------|------|
| feistel_b64_k32_r8 | SAT | 15.35s |
| random_v110_s1 | UNSAT | 6.86s |
| random_v130_s3 | SAT | 52.48s |

**PAR-2: 794.69 (3/6 solved)** — ~0.2% improvement from fat LTO + native CPU targeting.

### Iteration 2: Eliminate Vec Allocation in clause_state (MAJOR WIN)

The original `clause_state()` allocated a `Vec<i32>` on every call to collect unassigned literals. Replaced with a simple counter + single literal variable. Also added early return when 2+ unassigned literals found (no need to scan the rest of the clause).

**Before:**
```rust
fn clause_state(&self, clause: &[i32]) -> ClauseState {
    let mut unassigned = Vec::new();       // heap alloc every call!
    for &lit in clause { ... unassigned.push(lit) ... }
}
```

**After:**
```rust
fn clause_state(&self, clause: &[i32]) -> ClauseState {
    let mut unassigned_count = 0u32;
    let mut unassigned_lit = 0i32;
    for &lit in clause {
        ...
        if v == UNASSIGNED {
            unassigned_count += 1;
            if unassigned_count == 1 { unassigned_lit = lit; }
            else { return ClauseState::Undetermined; }  // early exit!
        }
    }
}
```

| Instance | Result | Time |
|----------|--------|------|
| feistel_b64_k32_r8 | SAT | 4.41s |
| random_v110_s1 | UNSAT | 1.88s |
| random_v130_s3 | SAT | 14.55s |
| random_v140_s1 | UNSAT | 31.46s |

**PAR-2: 532.30 (4/6 solved)** — 33% improvement. Solved random_v140_s1 which was previously timing out. This was by far the most impactful optimization: `clause_state` is called millions of times in the hot loop, so eliminating its per-call heap allocation was transformative.

### Iteration 3: u8 Assignment Array + Inline Annotations

Replaced `Vec<Option<bool>>` with `Vec<u8>` using constants `UNASSIGNED=0, TRUE=1, FALSE=2`. Added `#[inline(always)]` to `lit_value`, `assign`, `unassign`, `clause_state`.

**PAR-2: ~540 (4/6 solved)** — Marginal improvement, within measurement noise. The `Option<bool>` niche optimization already made it 1 byte, so the representation change was neutral. The inline annotations help avoid function call overhead in the tight inner loop.

### Iteration 4: Box<[i32]> Clause Storage

Changed clause storage from `Vec<Vec<i32>>` to `Vec<Box<[i32]>>`. Saves 8 bytes per clause (no capacity field) and signals to the compiler that clauses are immutable-length.

**PAR-2: ~539 (4/6 solved)** — Marginal improvement.

### Other Optimizations Applied

- **Pre-allocated trail** with `Vec::with_capacity(num_vars)` — avoids repeated reallocations during DPLL recursion
- **`std::mem::take`** in parser instead of `clone()` + `clear()` — eliminates a redundant heap allocation per clause during parsing
- **Removed redundant `all_satisfied` check** — replaced with targeted `has_conflict` only when all variables assigned

### Final Result

| Instance | Type | Vars | Clauses | Result | Time |
|----------|------|------|---------|--------|------|
| feistel_b64_k32_r8 | crypto | 1120 | 3968 | SAT | 5.39s |
| feistel_b64_k32_r10 | crypto | 1376 | 4928 | TIMEOUT | >120s |
| feistel_b64_k32_r12 | crypto | 1632 | 5888 | TIMEOUT | >120s |
| random_v110_s1 | 3-SAT | 110 | 469 | UNSAT | 2.10s |
| random_v130_s3 | 3-SAT | 130 | 555 | SAT | 16.23s |
| random_v140_s1 | 3-SAT | 140 | 597 | UNSAT | 35.31s |

**Final PAR-2: 539.03 (4/6 solved)**

### Summary

| Metric | Baseline | Optimized | Improvement |
|--------|----------|-----------|-------------|
| PAR-2 | 796.29 | 539.03 | 32.3% better |
| Solved | 3/6 | 4/6 | +1 instance |
| feistel_b64_k32_r8 | 15.61s | 5.39s | 2.9x faster |
| random_v110_s1 | 6.86s | 2.10s | 3.3x faster |
| random_v130_s3 | 53.83s | 16.23s | 3.3x faster |
| random_v140_s1 | TIMEOUT | 35.31s | newly solved |

### Approaches Tried and Reverted

- **Flat clause storage** (`Vec<i32>` + offsets): No improvement for these instance sizes; the pointer indirection through offsets offset the cache locality benefit.
- **Shared trail across recursion**: Regression (568 PAR-2). The push/pop bookkeeping overhead exceeded the allocation savings.
- **Restart scan after each propagation**: Major regression (667 PAR-2). Batching all unit propagations in one pass before restarting is far more efficient.
- **XOR trick for polarity flip**: Regression (558 PAR-2). Branch predictor handles the comparison version better.
- **Unsafe get_unchecked indexing**: Neutral — the compiler already eliminates bounds checks with `#[inline(always)]` and LTO.
